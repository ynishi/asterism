//! # asterism-ui — Asterism desktop UI (Tauri v2 backend)
//!
//! ## Role
//!
//! Tauri v2 + Rust + Svelte 5 desktop UI. The grid is dense, hovers pull
//! up a small constellation burst, and there is no persistent chat
//! surface (a Command-K palette is left as future work).
//!
//! ## Design intent
//!
//! - Local-first — no cloud dependencies; everything ships with the app.
//! - Grid density over card size; a hover surfaces a few related items
//!   in a side panel rather than growing the card.
//! - Register / tone is expressed through persona accent colours and
//!   cover-text typography.
//! - Image binaries are served through the Tauri v2 asset protocol
//!   (`convertFileSrc()`), never through IPC.
//!
//! ## `assetProtocol` scope (`tauri.conf.json`)
//!
//! The scope is `$HOME/**` and nothing else. Every widened alternative
//! was considered and rejected:
//!
//! - It cannot be narrowed to the profile home. An asset's `locator` is
//!   wherever the user's file already lives — `~/Pictures`, `~/Desktop`,
//!   a project directory — and `convertFileSrc(locator)` is the only
//!   path by which the grid, the detail pane and the wallpaper theme
//!   read an original. Narrowing to `~/.asterism/**` would leave every
//!   in-place import unreadable.
//! - It does not need `/tmp/**` or `/private/tmp/**`. Those entries were
//!   here for dropped macOS screenshots, but [`commands::rehome_dropped_path`]
//!   copies any TEMP-rooted drop under `$HOME` *before* the locator is
//!   persisted, precisely so the durable copy lands inside this scope.
//!   Keeping them granted read access to two world-writable directories
//!   for a path the app no longer produces.
//!
//! An original that lives outside `$HOME` (an external volume,
//! `/Users/Shared`) is consequently not readable over this protocol —
//! unchanged by the narrowing above, since neither removed entry
//! covered those paths either.

#![warn(missing_docs)]

pub mod commands;
pub mod error;
pub mod provider_sign_in;
pub mod state;
pub mod stored_connection;

/// Startup mode parsed from the process arguments.
///
/// `--headless` runs the same backend + HTTP serve without opening a
/// webview window (a single core serving both); the
/// default keeps the desktop window. `--port <N>` (or `--port=<N>`)
/// overrides the loopback listen port. Any other argument is ignored so
/// `tauri dev` can pass its own flags without breaking startup.
struct CliOpts {
    headless: bool,
    port: u16,
}

/// Parses `--headless` / `--port <N>` from `std::env::args`, ignoring
/// every other token. Kept dependency-free on purpose (no
/// `tauri-plugin-cli`): the surface is two flags.
fn parse_cli() -> CliOpts {
    let mut headless = false;
    let mut port = asterism_infra::paths::active_profile()
        .map(|profile| profile.default_http_port())
        .unwrap_or(8989);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => headless = true,
            "--port" => {
                if let Some(value) = args.next()
                    && let Ok(parsed) = value.parse::<u16>()
                {
                    port = parsed;
                }
            }
            other => {
                if let Some(value) = other.strip_prefix("--port=")
                    && let Ok(parsed) = value.parse::<u16>()
                {
                    port = parsed;
                }
                // Any other argument (including `tauri dev` extras) is
                // ignored so unknown flags never abort startup.
            }
        }
    }
    CliOpts { headless, port }
}

/// Tauri v2 app entrypoint (mobile-ready).
///
/// The setup hook initialises the whole backend (open the SQLite isle,
/// run migrations, start the job engine, wire up services), manages the
/// resulting `AppState`, opens the desktop window (unless `--headless`),
/// and spawns the loopback HTTP server that mirrors the Tauri command
/// surface. Command handlers stay thin — one per use-case call — and
/// live in the `commands` module.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before the builder, so a failure inside `setup` is already
    // covered. Replaces `tauri-plugin-log`, which only ran in debug
    // builds and therefore left a bundled app with no diagnostics at
    // all — records now also persist to `diag_log`, which outlives the
    // terminal the app was launched from.
    asterism_infra::observe::install();
    let opts = parse_cli();

    // The opener is how a sign-in through a team's identity provider
    // (#163) hands the person's default browser the page on the team
    // server. The backend calls it and nothing in the webview does,
    // which is why the capability file grants the webview none of the
    // plugin's commands: a capability is what the webview may reach
    // over IPC, and the Rust call is not that. Under the `wdio`
    // feature the call is compiled out and the plugin stays registered
    // — the suite is the browser, and the reason sits at the call.
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());

    // WebDriver surface for `just ui-e2e`, behind the `wdio` feature.
    // The feature is what keeps this out of a shipped app: a default
    // build does not compile the plugins at all, so the W3C server —
    // remote control over the whole webview, every DOM read and
    // arbitrary JS — cannot be reached even by setting an env var.
    // macOS ships no WKWebView driver, which is why the server has to
    // run inside the app rather than alongside it.
    #[cfg(feature = "wdio")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());

    builder
        .setup(move |app| {
            let handle = app.handle().clone();
            // Init failure (typically the tantivy writer lock held by an
            // already-running core) must not surface as a Tauri setup
            // panic/abort — exit cleanly so the second instance reads as
            // "another core is running", same as the port-bind guard.
            let (app_state, server_ctx) = match tauri::async_runtime::block_on(state::init(handle))
            {
                Ok(pair) => pair,
                Err(err) => {
                    tracing::error!(
                        event = "diag.core_init.failed",
                        error = ?err,
                        "core init failed (another core may already own this ASTERISM_HOME)"
                    );
                    // Also on stderr, verbatim: the database never
                    // opened, so the record above has nowhere
                    // durable to go, and this is the last thing the
                    // user sees before the process exits.
                    eprintln!("asterism: core init failed: {err:#}");
                    std::process::exit(1);
                }
            };
            use tauri::Manager;

            // Container cover backfill. A container takes its cover from
            // its earliest member, which ingest can only trigger for
            // members arriving after that wiring existed — a container
            // that was already whole never gets one. The job is
            // idempotent (it enqueues only for containers still lacking
            // a cover, and `cover_gen` no-ops once one is set), so
            // running it every start settles to a single empty scan.
            // Enqueue failure is not fatal: the next start retries.
            {
                let assets = app_state.asset_service.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = assets.rebuild_sessions().await {
                        tracing::warn!(
                            event = "diag.session_rebuild.enqueue_failed",
                            error = ?err,
                            "container cover backfill was not enqueued"
                        );
                    }
                });
            }

            app.manage(app_state);

            // Window: created programmatically (the static
            // `tauri.conf.json` `app.windows` declaration is now `[]`) so
            // `--headless` can skip it. `WebviewUrl::default()` resolves
            // to `devUrl` in dev and `frontendDist` in a bundle, so HMR /
            // dev behaviour is unchanged.
            if !opts.headless {
                use tauri::{WebviewUrl, WebviewWindowBuilder};
                // Drag-and-drop is enabled by default (the old static
                // `dragDropEnabled: true` was a no-op restatement).
                let profile = asterism_infra::paths::active_profile()?;
                let title = match profile {
                    asterism_infra::paths::DataProfile::Dogfood => "Asterism".to_string(),
                    other => format!("Asterism — {}", other.as_str().to_ascii_uppercase()),
                };
                // The size is the UI's own number rather than a taste.
                // `DetailPane.svelte`'s `.detail-panel` is
                // `width: min(96vw, 1200px)`, so the panel reaches the
                // width it was drawn for only past 1250px of window; at
                // the 800 this used to open at it was clamped to 768,
                // about three fifths of what it asks for. `max-height:
                // 96vh` meets the same wall at 600 tall.
                //
                // Deliberately neither maximized nor full screen. A full
                // screen window on macOS gets a Space of its own, so
                // reaching anything beside it — a ComfyUI tab, a
                // terminal — costs a Space switch; a large ordinary
                // window stays where the rest of the work is.
                //
                // The floor is the shell's: `180px` of sidebar beside a
                // grid of `minmax(180px, 1fr)` cards, which needs room
                // for three of them before the arrangement stops being
                // one. Without it the window dragged down to nothing.
                // The frontend declares `color-scheme: dark` and means
                // it: there is one value set and no light one to fall
                // back to. Two pieces of this window are the operating
                // system's rather than the stylesheet's, and both have
                // to be told the same thing or the app arrives with a
                // seam down the middle of it.
                //
                // `theme` is the title bar. Left as `None` it follows
                // the OS, so an app that is always dark wore a light
                // title bar on a light Mac — visible in every
                // screenshot and the one part of the window CSS cannot
                // reach.
                //
                // `background_color` is what the window is before the
                // webview has painted anything. On macOS it reaches the
                // window layer only (the webview layer is not
                // implemented there), which is exactly the layer that
                // shows during launch. `#16151c` is `--surface` from
                // `app.css` — the value is duplicated because a
                // stylesheet cannot hand a colour to a window builder,
                // and this is the only place that copy exists.
                WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                    .title(title)
                    .inner_size(1280.0, 860.0)
                    .min_inner_size(960.0, 640.0)
                    .resizable(true)
                    .theme(Some(tauri::Theme::Dark))
                    .background_color(tauri::window::Color(0x16, 0x15, 0x1c, 255))
                    .build()?;
            }

            // macOS: keep a headless process out of the Dock / app switcher.
            #[cfg(target_os = "macos")]
            if opts.headless {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            // HTTP serve (both modes). One core per machine: bind 127.0.0.1
            // on the resolved port and serve the shared router. A bind
            // failure means another core already holds the port — in
            // windowed mode we warn and keep running as a plain UI; in
            // headless mode serving is the only reason to exist, so we exit.
            let port = opts.port;
            let headless = opts.headless;
            tauri::async_runtime::spawn(async move {
                let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
                match tokio::net::TcpListener::bind(addr).await {
                    Ok(listener) => {
                        tracing::info!(
                            event = "diag.http.serving",
                            url = %format!("http://{addr}/asterism/health"),
                            headless,
                            "serving loopback HTTP"
                        );
                        let router = asterism_server::http::router(server_ctx);
                        if let Err(e) = axum::serve(listener, router).await {
                            tracing::error!(
                                event = "diag.http.serve_failed",
                                error = %e,
                                "http serve error"
                            );
                        }
                    }
                    Err(e) => {
                        if headless {
                            tracing::error!(
                                event = "diag.http.bind_refused",
                                %addr,
                                error = %e,
                                "cannot bind; another core already holds the port — exiting"
                            );
                            std::process::exit(1);
                        } else {
                            tracing::warn!(
                                event = "diag.http.bind_refused",
                                %addr,
                                error = %e,
                                "cannot bind (another core holds the port); \
                                 continuing as UI without HTTP"
                            );
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::active_profile,
            commands::list_personas,
            commands::list_persona_asset_counts,
            commands::list_modality_asset_counts,
            commands::list_format_asset_counts,
            commands::list_color_asset_counts,
            commands::list_duplicate_groups,
            commands::list_duplicate_conflicts,
            commands::resolve_duplicate_conflict,
            commands::merge_assets,
            commands::list_modalities,
            commands::create_modality,
            commands::update_modality,
            commands::delete_modality,
            commands::list_series_strategies,
            commands::create_series_strategy,
            commands::update_series_strategy,
            commands::delete_series_strategy,
            commands::list_settings,
            commands::set_setting,
            commands::reset_setting,
            commands::register_persona,
            commands::archive_persona,
            commands::trash_persona,
            commands::restore_persona,
            commands::purge_persona,
            commands::reorder_personas,
            commands::get_persona_theme,
            commands::set_persona_theme,
            commands::delete_persona_theme,
            commands::get_persona_profile,
            commands::set_persona_profile,
            commands::delete_persona_profile,
            commands::add_asset,
            commands::paste_image_import,
            commands::rehome_dropped_path,
            commands::add_asset_batch,
            commands::update_asset_meta,
            commands::update_asset_meta_batch,
            commands::trash_asset,
            commands::restore_asset,
            commands::purge_asset,
            commands::empty_trash,
            commands::list_assets,
            commands::list_asset_index,
            commands::hydrate_cards,
            commands::search_assets,
            commands::search_asset_ids,
            commands::random_assets,
            commands::asset_detail,
            commands::asset_edges,
            commands::asset_constellation,
            commands::asset_provenance,
            commands::asset_lineage,
            commands::asset_declare_provenance,
            commands::asset_declare_meta,
            commands::asset_declare_source_type,
            commands::asset_source_type,
            commands::asset_video_preview,
            commands::rebuild_edges,
            commands::rebuild_index,
            commands::rescan_duplicates,
            commands::remeasure_dims,
            commands::organize_by_location,
            commands::get_asset_thumb,
            commands::get_asset_thumbs,
            commands::jobs_stats,
            commands::record_event,
            commands::record_diag,
            commands::list_events,
            commands::list_observations,
            commands::list_streams,
            commands::list_tag_counts,
            commands::attach_tag,
            commands::list_tag_suggestions,
            commands::accept_tag_suggestion,
            commands::reject_tag_suggestion,
            commands::visual_model_status,
            commands::head_status,
            commands::train_tag_head,
            commands::pull_tag_head,
            commands::detach_tag,
            commands::attach_tag_batch,
            commands::detach_tag_batch,
            commands::promote_tag_to_group,
            commands::rename_tag,
            commands::delete_tag,
            commands::merge_tags,
            commands::list_sessions,
            commands::rebuild_sessions,
            commands::get_session,
            commands::rename_session,
            commands::patch_session_metadata,
            commands::delete_session,
            commands::list_groups,
            commands::create_group,
            commands::trash_group,
            commands::restore_group,
            commands::purge_group,
            commands::add_asset_to_group,
            commands::remove_asset_from_group,
            commands::batch_group_membership,
            commands::merge_groups,
            commands::groups_of_asset,
            commands::reorder_group_assets,
            commands::rename_group,
            commands::move_group_to_dir,
            commands::link_group,
            commands::unlink_group,
            commands::list_group_links,
            commands::reorder_group_children,
            commands::list_dirs,
            commands::create_dir,
            commands::rename_dir,
            commands::move_dir,
            commands::delete_dir,
            commands::asset_texts,
            commands::list_snapshots_containing,
            commands::create_snapshot,
            commands::promote_snapshot_to_group,
            commands::promote_volatile_selection,
            commands::get_snapshot,
            commands::snapshot_members,
            commands::dispatch_run,
            commands::redispatch,
            commands::create_query_group,
            commands::update_query_group_query,
            commands::create_dispatch,
            commands::get_dispatch,
            commands::list_dispatch,
            commands::list_exporters,
            commands::list_asset_comments,
            commands::post_asset_comment,
            commands::edit_asset_comment,
            commands::delete_asset_comment,
            commands::list_material_marks,
            commands::post_material_mark,
            commands::edit_material_mark,
            commands::delete_material_mark,
            commands::list_material_layers,
            commands::list_chapter_marks,
            commands::create_material_layer,
            commands::set_default_material_layer,
            commands::delete_material_layer,
            commands::post_chapter_mark,
            commands::edit_chapter_mark,
            commands::delete_chapter_mark,
            commands::list_threads,
            commands::get_thread,
            commands::create_thread,
            commands::archive_thread,
            commands::delete_thread,
            commands::list_thread_messages,
            commands::append_thread_message,
            commands::delete_thread_message,
            commands::open_forge_line,
            commands::list_forge_lines,
            commands::get_forge_line,
            commands::get_forge_line_states,
            commands::rename_forge_line,
            commands::set_forge_line_strategy,
            commands::archive_forge_line,
            commands::reopen_forge_line,
            commands::discard_forge_line,
            commands::list_forge_strategies,
            commands::open_forge_pursuit,
            commands::get_forge_pursuit,
            commands::push_forge_round,
            commands::resolve_forge_pursuit,
            commands::close_forge_pursuit,
            commands::get_forge_pursuit_collisions,
            commands::get_forge_pursuit_behind,
            commands::list_forge_pursuit_children,
            commands::list_forge_pursuits_of_line,
            commands::open_forge_thread,
            commands::get_forge_thread,
            commands::say_in_forge_thread,
            commands::amend_forge_message,
            commands::rename_forge_thread,
            commands::list_forge_threads_about,
            // Lines a team hosts. Their own block for the reason they
            // get their own panel: the source is a server, not this
            // machine (#148 decision 16).
            commands::connect_team_server,
            commands::disconnect_team_server,
            commands::team_server_session,
            commands::team_server_identity,
            // The other way in (#163): what the server offers besides
            // a password, and the sign-in that goes through the browser
            // and comes back on loopback.
            commands::team_auth_provider,
            commands::connect_team_server_provider,
            commands::cancel_provider_sign_in,
            // The one credential this machine may keep, and what it
            // costs to keep it (#204). `stored_connection` owns both
            // stores; this block is the verbs a window reaches them
            // through by name, and the two connects above reach them
            // through the mint.
            commands::stored_team_connection,
            commands::connect_team_server_stored,
            commands::list_team_device_tokens,
            commands::revoke_team_device_token,
            commands::list_shared_lines,
            commands::shared_line_states,
            commands::shared_line_history,
            commands::shared_line_pursuits,
            commands::shared_pursuit,
            commands::shared_pursuit_collisions,
            commands::shared_pursuit_behind,
            commands::open_shared_pursuit,
            commands::push_shared_round,
            commands::close_shared_pursuit,
            commands::clone_shared_entry,
            commands::promote_asset_to_team,
            commands::publish_line_to_team,
            commands::team_ledger_page,
            commands::my_teams,
            commands::team_roster,
            commands::create_team,
            commands::rename_team,
            commands::invite_team_member,
            commands::invite_team_member_by_login,
            commands::remove_team_member,
            commands::grant_team_owner,
            commands::revoke_team_owner,
            commands::leave_team,
            commands::delete_team,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
