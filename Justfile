set shell := ["zsh", "-cu"]

project_root := justfile_directory()
ui_dir := project_root + "/crates/asterism-ui"
dev_home := project_root + "/workspace/runtime/dev"
dogfood_home := env_var("HOME") + "/.asterism/profiles/dogfood"
bench_home := env_var("HOME") + "/.asterism/profiles/bench"
dogfood_app := project_root + "/target/release/bundle/macos/Asterism.app"

# Show the available commands.
default:
    @just --list

# Build the LGPL-clean ffmpeg sidecar the bundle carries (idempotent;
# exits fast once built). Every recipe that runs `tauri build` or
# `tauri dev` depends on this: bundle.externalBin names the binary,
# and Tauri fails the build when it is missing. externalBin is declared
# only in the CLI merge configs (tauri.dev/e2e/bundle.conf.json), not
# the base tauri.conf.json, so plain cargo compiles (check / clippy /
# test on a fresh target dir or worktree) never require the sidecar —
# only the tauri CLI paths below do. Lives under target/ so
# `cargo clean` wipes it like any other artifact — this recipe
# rebuilds it (~2-4 min once per clean).
[group('app')]
ffmpeg-sidecar:
    "{{ project_root }}/scripts/build-ffmpeg-sidecar.sh"

# Run the disposable Dev app.
[group('app')]
dev: ffmpeg-sidecar
    cd "{{ ui_dir }}" && npm run app:dev

# Build and launch the production-shaped Dogfood app.
[group('app')]
dogfood: dogfood-build
    open "{{ dogfood_app }}"

# Launch an already-built Dogfood app without rebuilding it.
[group('app')]
dogfood-open:
    @test -d "{{ dogfood_app }}" || (echo "Dogfood app is not built; run: just dogfood-build" >&2; exit 1)
    open "{{ dogfood_app }}"

# Restart the already-built Dogfood app so a fresh build takes over the
# port — the shell-side twin of the MCP proxy's `app_restart` tool.
# Graceful path first: POST /asterism/admin/shutdown (the serving
# process exits itself, no name-matching involved). A serving build
# that predates the endpoint answers non-200, so fall back to a
# full-path pkill for that one transition. Then relaunch, poll
# /asterism/health, and print the health body — `git_sha` + `pid` are
# how you check the *new* build actually took over instead of trusting
# the relaunch.
[group('app')]
dogfood-restart:
    @test -d "{{ dogfood_app }}" || (echo "Dogfood app is not built; run: just dogfood-build" >&2; exit 1)
    @code=$(curl -sS -o /dev/null -w "%{http_code}" -X POST http://127.0.0.1:8989/asterism/admin/shutdown 2>/dev/null || true); \
    if [ "$code" != "200" ]; then pkill -f "Asterism.app/Contents/MacOS" 2>/dev/null || true; fi
    @for i in {1..20}; do \
        code=$(curl -sS -o /dev/null -w "%{http_code}" http://127.0.0.1:8989/asterism/health 2>/dev/null || true); \
        if [ "$code" != "200" ]; then break; fi; \
        sleep 0.2; \
    done
    open "{{ dogfood_app }}"
    @# 20s wall-clock ceiling matches the launchd-restart pattern; a
    @# cold start on Apple Silicon warms up in ~2-3s.
    @for i in {1..40}; do \
        code=$(curl -sS -o /dev/null -w "%{http_code}" http://127.0.0.1:8989/asterism/health 2>/dev/null || true); \
        if [ "$code" = "200" ]; then \
            echo "Asterism app is up (waited ${i}x500ms):"; \
            curl -sS http://127.0.0.1:8989/asterism/health; echo; exit 0; \
        fi; \
        sleep 0.5; \
    done; \
    echo "Asterism app did not answer /asterism/health within 20s" >&2; exit 1

# Build the release `asterism-server` binary that MCP clients spawn as
# the stdio proxy (`asterism-server mcp`). The proxy forwards tool and
# resource schemas from the running app, so app rebuilds do NOT require
# rebuilding this binary — only changes to the proxy itself do.
[group('app')]
mcp-proxy-build:
    cargo build --release -p asterism-server

# Build the production-shaped Dogfood app without launching it.
# The trailing assert is the teeth for the config split (2026-08-04):
# externalBin rides tauri.bundle.conf.json via `--config` merge, and if
# that merge ever stops reaching tauri-build the bundler would silently
# ship an app without the sidecar — runtime would fall back to whatever
# ffmpeg the host carries instead of failing loudly here.
[group('app')]
dogfood-build: ffmpeg-sidecar
    cd "{{ ui_dir }}" && npm run app:dogfood:build
    @test -x "{{ dogfood_app }}/Contents/MacOS/ffmpeg" || (echo "bundle is missing the ffmpeg sidecar — tauri.bundle.conf.json externalBin merge did not reach tauri-build" >&2; exit 1)

# Run the large-fixture Bench app.
[group('app')]
bench: ffmpeg-sidecar
    cd "{{ ui_dir }}" && npm run app:bench

# Run the Dev backend on http://127.0.0.1:18989.
[group('headless')]
dev-headless:
    ASTERISM_PROFILE=dev ASTERISM_HOME="{{ dev_home }}" cargo run -q -p asterism-ui -- --headless --port 18989

# Run the Dogfood backend on http://127.0.0.1:8989.
[group('headless')]
dogfood-headless:
    ASTERISM_PROFILE=dogfood ASTERISM_HOME="{{ dogfood_home }}" cargo run --release -q -p asterism-ui -- --headless --port 8989

# Run the Bench backend on http://127.0.0.1:28989.
[group('headless')]
bench-headless:
    ASTERISM_PROFILE=bench ASTERISM_HOME="{{ bench_home }}" cargo run -q -p asterism-ui -- --headless --port 28989

# Idempotently initialize or migrate the Dev profile.
[group('profile')]
dev-init:
    ASTERISM_PROFILE=dev ASTERISM_HOME="{{ dev_home }}" cargo run -q -p asterism-server -- init

# Idempotently initialize or migrate the Dogfood profile.
[group('profile')]
dogfood-init:
    ASTERISM_PROFILE=dogfood ASTERISM_HOME="{{ dogfood_home }}" cargo run -q -p asterism-server -- init

# Idempotently initialize or migrate the Bench profile.
[group('profile')]
bench-init:
    ASTERISM_PROFILE=bench ASTERISM_HOME="{{ bench_home }}" cargo run -q -p asterism-server -- init

# Run the unified importer CLI (for example: just import tape --help).
[group('import')]
import *args:
    cargo run -q -p asterism-importer -- {{ args }}

# Generate the seeded corpus for a preset (s = 5,000 files / m = 12,000 /
# l = manifest only). Idempotent: a matching manifest short-circuits, and
# an interrupted run resumes over the files already written.
#
# `--release` throughout this group is not a preference. Every one of
# these recipes is dominated by procedural pixel work and PNG / JPEG
# encoding, which a debug build runs several times slower — a debug
# corpus run is the difference between minutes and an hour.
[group('bench')]
bench-corpus preset seed="42":
    cargo run --release -q -p asterism-benchgen -- corpus --preset {{ preset }} --seed {{ seed }}

# Seed the metadata tier (110,000 rows + 256 px thumbnails) straight into
# the bench profile's database. No server and no files: this is the tier
# behind the cold-load and 10k-per-group scroll measurements.
#
# Writes nowhere but `profiles/bench` — the command refuses any other
# database. Assets are not idempotent (locators are unique per index), so
# run `just bench-reset` before re-seeding.
[group('bench')]
bench-seed-l seed="42":
    cargo run --release -q -p asterism-benchgen -- seed-meta --preset l --seed {{ seed }}

# Load the file tier into a running bench server (`just bench-headless`),
# so the import path — hashing every byte, generating every thumbnail —
# does the work this tier exists to measure.
[group('bench')]
bench-load preset seed="42":
    cargo run --release -q -p asterism-benchgen -- load-file --preset {{ preset }} --seed {{ seed }}

# Empty the bench profile's database so the next seed / load starts from
# a known state.
#
# Deletes the SQLite trio and the Tantivy index directory, and nothing
# else: the `.asterism-profile` marker stays (removing it would let the
# next open bind the directory to a different profile), and the corpus
# directory is untouched — regenerating 18 GB of PNGs to re-run a load is
# not what "reset the database" should mean.
[group('bench')]
bench-reset:
    @test -f "{{ bench_home }}/.asterism-profile" || (echo "{{ bench_home }} is not an initialised bench profile; run: just bench-init" >&2; exit 1)
    rm -f "{{ bench_home }}/asterism.db" "{{ bench_home }}/asterism.db-wal" "{{ bench_home }}/asterism.db-shm"
    rm -rf "{{ bench_home }}/tantivy"
    @echo "bench profile reset: {{ bench_home }} (corpus dir untouched)"

# Measure an import end to end: registration + the job drain behind it.
#
# Run it in this order, and not otherwise:
#
#   1. just bench-corpus <preset>   (once per seed; ~17 min for s)
#   2. just bench-reset             (a load into a populated profile is
#                                    not an import measurement)
#   3. just bench-headless          (separate terminal, port 28989)
#   4. just bench-measure-import <preset>
#
# The subcommand deliberately does not reset the profile for you: a tool
# that deletes a database to make its own number valid is not one to
# hand someone in a hurry. It writes
# `workspace/bench-results/<UTC>-import-<preset>.json` — registration
# passes, the drain timeline (5 s polls of `/asterism/jobs/depth`), and
# an RSS sample per poll.
[group('bench')]
bench-measure-import preset seed="42":
    cargo run --release -q -p asterism-benchgen -- measure-import --preset {{ preset }} --seed {{ seed }}

# Measure the pursuit membership reads (#29) against a self-seeded
# temp profile at the documented asset scale. Self-contained: no
# server, no bench profile, no reset dance — the command seeds a
# throwaway database and measures through the real repository
# adapters. This is the receipt for the index-seek bet the V80
# lookup columns make, and the number that decides whether a
# job-built materialised projection is ever needed. The result file
# lands beside the other measurements, under
# `workspace/bench-results/pursuit-view-<stamp>.json`.
[group('bench')]
bench-measure-pursuit assets="100000":
    cargo run --release -q -p asterism-benchgen -- measure-pursuit --assets {{ assets }}

# Measure the first listing a freshly started backend answers (issue
# d47b0759), plus the warm repeat.
#
# "Cold" is a property of the process that answers, so **restart the
# bench server immediately before this** — stop `just bench-headless`
# and start it again. Running this against a server that has already
# served the grid measures a warm cache twice.
#
# Writes `workspace/bench-results/<UTC>-cold.json`. Note that the
# server-side breakdown (`GET /asterism/perf`) is written only under the
# dev profile, so a bench-profile run records wall clock only and says
# so in the file.
[group('bench')]
bench-measure-cold:
    cargo run --release -q -p asterism-benchgen -- measure-cold

# Drive the grid: one constant-speed pass, then N jumps of five to ten
# screenfuls each, reporting thumb latency per 10 jumps.
#
# **The jump phase is the one that matters.** The complaint being
# reproduced is: open a folder of several thousand images, flick the
# scrollbar a few hundred cards at a time, and the first few screenfuls
# paint in about a second while later ones take five to ten. That is a
# function of how many screenfuls have been asked for, not of how long
# the app has been open, so the phase is counted in jumps and the
# windows are sampled in jumps.
#
# The distance is what makes it work: five to ten viewports lands
# entirely outside what was on screen, so every jump asks for a fresh
# screenful while the previous ones are still in flight — nothing
# cancels them (`thumb.svelte.ts:202`), each retries on a 250→4000 ms
# backoff, and a card that exhausts its budget is never fetched again
# (`:210-213`). An earlier version walked ±3 viewports and reported "no
# degradation"; it had simply stayed inside the cache.
#
# Prerequisites, in this order:
#
#   1. just bench-init                 (once)
#   2. just bench-reset && just bench-seed-l   (~6 min; the L preset is
#                                       what "10k per group" means)
#   3. just bench-scroll [jumps] [seed]
#
# No bench *server* is needed — this drives the desktop app itself,
# which opens the same profile directly.
#
# Two things separate this from `ui-e2e`, and both are the point:
#
#   * `VITE_BENCH=1`. `tauri build --debug` embeds a **production**
#     Vite build, where `thumb-perf.ts`'s `DEV` gate is shut — so
#     without this flag the run completes and reports zero fetches.
#     The scenario refuses to start if the handle is missing, rather
#     than publishing that zero.
#   * `wdio.bench.conf.ts`. Its own spec glob (`e2e-bench/`), its own
#     port (19898), and `ASTERISM_PROFILE=bench` with no
#     `ASTERISM_HOME`, so the app resolves `~/.asterism/profiles/bench`
#     and its `.asterism-profile` marker check stays in play. The
#     scenario re-checks the profile from the inside (every persona
#     must carry the `bench-persona-` prefix) before it scrolls.
#
# Not in `check`: minutes by construction. Writes
# `workspace/bench-results/<UTC>-scroll.json`; frames land in
# `workspace/test-logs/bench-screens/<UTC>/`.
[group('bench')]
bench-scroll jumps="200" seed="42": ffmpeg-sidecar
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{ ui_dir }}"
    # Same build shape as `ui-e2e` (see its comment for why `tauri
    # build` rather than `cargo build`), plus the bench gate.
    VITE_WDIO=1 VITE_BENCH=1 npx tauri build --debug --no-bundle \
        --features wdio --config src-tauri/tauri.e2e.conf.json
    # `--spec` even with one scenario present: the conf's glob matches
    # everything under `e2e-bench/`, so a second scenario added later
    # would silently join this run on a profile staged for this one.
    BENCH_JUMPS="{{ jumps }}" BENCH_CORPUS_SEED="{{ seed }}" \
        npx wdio run wdio.bench.conf.ts --spec ./e2e-bench/bench-scroll.spec.ts

# Regenerate the LLM-facing doc artifacts under docs/aidoc/.
#
# cargo-aidoc (https://github.com/ynishi/cargo-aidoc) projects rustdoc
# JSON into committed markdown: llms.txt, a per-crate index.md carrying
# the public-module list, and per-module reference pages. That committed
# copy is the module inventory this repository maintains — domain/mod.rs
# stopped hand-listing its submodules after the list shipped 15 modules
# stale (27 of 42, #25). A generated inventory cannot rot the same way
# because `aidoc-check` fails when it drifts from the tree.
#
# Two prerequisites, stated here because the recipe outlives any PR:
# (1) a nightly toolchain — the pipeline shells out to `cargo +nightly
# rustdoc --output-format json` per crate, which is why neither aidoc
# recipe joins `check`: this workspace's toolchain floats (no
# rust-toolchain.toml) and a machine without nightly must still be able
# to run the full gate. (2) cargo-aidoc newer than the 0.1.0 release,
# which fails on this workspace: it derived the rustdoc JSON filename
# from the package name, and asterism-ui's `[lib] name =
# "asterism_ui_lib"` (the Tauri bin/lib name split) breaks that
# assumption. Until a fixed version is on crates.io, install from the
# repo: `cargo install --git https://github.com/ynishi/cargo-aidoc
# cargo-aidoc`.
#
# `--title` pins the llms.txt H1. The tool's default is the checkout
# directory's basename, which from a worktree named after its branch
# would bake the branch slug into a committed artifact — and then
# `aidoc-check` reports drift from every other checkout.
#
# Run this after changing any public API or doc comment, and commit
# the diff.
aidoc:
    cargo aidoc --workspace-root "{{ project_root }}" --title asterism

# Fail when docs/aidoc/ no longer matches the tree (exit 2 on drift).
#
# The other half of #25's guard: adding a module without regenerating
# the committed artifacts is exactly the "addition skips the inventory"
# hole a hand-written list cannot detect — intra-doc links only break on
# removals. Same prerequisites as `aidoc`, same reason it stays outside
# `check`.
#
# Carries `allow-agent` on ui-e2e's half of that group's reasoning, not
# the "seconds long, writes nothing" half: a run costs minutes and
# writes rustdoc JSON under target/ in order to have something to
# compare (docs/aidoc/ itself is untouched in check mode), but it is
# the only surface that can check the inventory, so it is run
# deliberately rather than skipped.
[group('allow-agent')]
aidoc-check:
    cargo aidoc --workspace-root "{{ project_root }}" --check --strict --title asterism

# Run all Rust and frontend checks.
[group('check')]
check: rust-fmt-check rust-clippy bindings-check rust-test ui-test ui-check ui-build

# Fail when any Rust file is not rustfmt-clean.
#
# Neither `check` nor `rust-test` used to look at formatting, so drift
# accumulated invisibly on main until someone ran `cargo fmt` for an
# unrelated change and six files' worth of reformatting landed in their
# diff (2026-07-31, HEAD 1e290a9). Detection belongs here, at the same
# gate everything else passes through. The fix on failure is one
# command and one commit: `cargo fmt --all`, committed on its own.
#
# Carries `allow-agent` on the same terms as the frontend three: it reads
# the tree and writes nothing (`--check`), finishes in seconds, and an
# agent that cannot run it can only report that it did not check.
[group('check')]
[group('allow-agent')]
rust-fmt-check:
    cargo fmt --all -- --check

# Fail on any clippy warning, across every target (tests and examples
# included).
#
# Nothing ran clippy until 2026-08-02, so a toolchain bump (clippy 1.95)
# left 46 violations across 26 files and the linter useless: the first
# offending crate aborted the run before downstream crates were even
# checked. Restored in 68c51ec; this recipe is what keeps the workspace
# at zero from here on. On failure, fix the named lints — lint fixes
# stay in their own commit, like fmt.
#
# No ffmpeg-sidecar dependency: bundle.externalBin lives only in the
# CLI merge configs (tauri.dev/e2e/bundle.conf.json) since 2026-08-04,
# so a plain cargo compile of the asterism-ui build script never
# validates the sidecar path. Before that split, `--all-targets` on any
# fresh target dir (cargo clean, new worktree, agent recipe) aborted
# with a resource error that named no recipe (observed 2026-08-03 and
# again 2026-08-04 from a fresh worktree).
#
# Carries `allow-agent`: it writes nothing to the tree and its verdict is
# the one an agent has to answer for before handing work back. `rust-test`
# deliberately does not — minutes long, and handing it over invites a full
# suite where a narrow run was the right tool.
[group('check')]
[group('allow-agent')]
rust-clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Fail when the committed TypeScript bindings no longer match the contract.
#
# `crates/asterism-ui/src/bindings.ts` is generated *and* tracked:
# `src-tauri/build.rs` rewrites it from the `asterism-contract` types,
# and git carries a copy so that anything reading the frontend does not
# have to build Rust first. Nothing compared the two until this recipe.
# The gap is quiet by construction — whoever changes a contract type
# gets the regenerated file as a side effect of a cargo command, and if
# they do not commit it, every developer keeps building from a copy
# regenerated on their own machine while the repository's falls behind.
#
# The `touch` is what makes this a check rather than a coin flip.
# `tauri_build::build()` emits `cargo:rerun-if-changed` for
# `tauri.conf.json` and `capabilities/`, and a script that emits any
# `rerun-if` directive loses cargo's default of re-running whenever
# anything in the package changes — the rule `asterism-server/build.rs`
# states in its own words. In a warm tree with neither of those touched
# and no contract change to relink the script, `cargo check` would run
# no build script at all: `bindings.ts` would not be rewritten, the diff
# below would compare the committed file against itself, and this recipe
# would report a match it never computed. A gate that passes without
# checking is worse than no gate. Touching the script forces the run,
# and an mtime is not something git sees.
#
# Compared against `HEAD` rather than the index because the question is
# whether the copy this repository carries is stale. Staging a
# regenerated file does not answer that, so a staged-but-uncommitted fix
# still fails here — correctly, since nothing is committed yet.
#
# Deliberately fails instead of staging the regenerated file. A gate that
# edits the tree turns a verdict into a side effect, and the next reader
# cannot tell which of the two happened.
#
# This is the one `allow-agent` recipe that writes a file git tracks —
# the frontend three confine their output to the ignored `dist/`, and
# `rust-fmt-check` goes out of its way to use `--check`. It carries the
# annotation on the group's other stated reason rather than that one: an
# agent that cannot run it can only report that it did not check.
[group('check')]
[group('allow-agent')]
bindings-check:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{ project_root }}"
    bindings="crates/asterism-ui/src/bindings.ts"
    touch crates/asterism-ui/src-tauri/build.rs
    cargo check -p asterism-ui >/dev/null
    if ! git diff --quiet HEAD -- "$bindings"; then
        echo "$bindings differs from what asterism-contract generates." >&2
        echo >&2
        git --no-pager diff HEAD -- "$bindings" >&2
        echo >&2
        echo "Resolve by committing the regenerated file:" >&2
        echo "    git add $bindings && git commit" >&2
        exit 1
    fi
    echo "$bindings matches the contract"

# Run the Rust workspace tests, keeping the whole output.
#
# The only sanctioned way to run the full suite. `cargo test --workspace`
# by hand is what produced an unexplainable result on 2026-07-30: the run
# reported 455 passed / 1 failed against 472 expected, the operator had
# piped it through an aggregate `grep`, and the identity of the failing
# and the 16 unreported tests was gone. Three re-runs came back green, so
# there was nothing left to diagnose.
#
# Three things this does that a bare invocation does not:
#
#   1. keeps the full log (`workspace/test-logs/`, gitignored) so a
#      one-off failure survives the terminal
#   2. `--no-fail-fast`, so one binary failing does not hide the rest
#   3. checks that every launched binary reported a result — a binary
#      killed mid-run prints no `test result:` line, and its passed
#      tests simply vanish from any sum. That silent subtraction is the
#      exact shape of the 2026-07-30 observation.
[group('check')]
rust-test: rust-fmt-check
    #!/usr/bin/env bash
    set -uo pipefail
    # Point 3 above reads cargo's own output, so the output has to stay
    # plain. Coloured, `   Running` arrives as `ESC[1mESC[92m   Running`
    # and the anchored patterns below match nothing: the count comes back
    # 0 launched against a full set of reported binaries, and the check
    # fails over a suite that passed. That is what the first CI run on
    # this repository did — 0 / 81 under `1191 passed / 0 failed` — but
    # nothing about it was specific to CI: any terminal that turns colour
    # on for a pipe, or a `CARGO_TERM_COLOR=always` in someone's
    # environment, reaches the same place. The recipe owns the shape it
    # parses.
    export CARGO_TERM_COLOR=never
    log_dir="{{ project_root }}/workspace/test-logs"
    mkdir -p "$log_dir"
    log="$log_dir/rust-test-$(date +%Y%m%d-%H%M%S).log"
    cargo test --workspace --no-fail-fast 2>&1 | tee "$log"
    status=${PIPESTATUS[0]}

    launched=$(grep -cE '^ +(Running|Doc-tests)' "$log" || true)
    reported=$(grep -cE '^test result:' "$log" || true)
    passed=$(grep -E '^test result:' "$log" | awk '{p+=$4} END {print p+0}')
    failed=$(grep -E '^test result:' "$log" | awk '{f+=$6} END {print f+0}')

    echo
    echo "log:      $log"
    echo "binaries: $launched launched / $reported reported"
    echo "tests:    $passed passed / $failed failed"

    if [ "$launched" -ne "$reported" ]; then
        echo
        echo "MISSING: $((launched - reported)) binary/binaries never reported a result." >&2
        echo "Their tests are absent from the counts above — the totals are a floor," >&2
        echo "not a tally. The binaries that were launched but stayed silent:" >&2
        # Identity is the last field of a `Running` line (the
        # `(target/debug/deps/…)` path) or the crate on a `Doc-tests`
        # line. Not `$2`: that is the literal word `unittests`, which
        # would make every binary look like the same one.
        awk '
            /^ +Running /   { key = $NF; gsub(/[()]/, "", key); seen[key] = 1; last = key; next }
            /^ +Doc-tests / { key = "doc:" $2;                  seen[key] = 1; last = key; next }
            /^test result:/ { if (last != "") reported[last] = 1 }
            END { for (k in seen) if (!(k in reported)) print "  " k }
        ' "$log" >&2
        exit 1
    fi

    if [ "$failed" -ne 0 ]; then
        echo
        echo "FAILED tests (names, then the panic that produced each):" >&2
        # `test <name> ... FAILED` is one line per failure and survives
        # interleaving across binaries, unlike the trailing `failures:`
        # block which repeats per binary and lists names only.
        grep -E '^test .* FAILED$' "$log" | sed 's/^/  /' >&2 || true
        grep -E "panicked at" "$log" | sed 's/^/  /' | head -20 >&2 || true
        exit "$status"
    fi
    exit "$status"

# The three frontend recipes below also carry `allow-agent`, the group
# an lds-driven agent is allowed to run. They are the whole verification
# surface for UI work, so an agent without them can only report that it
# did not verify — which is what happened before this annotation existed.
# Safe to hand over for the reason the Rust suite is not: each is seconds
# long, reads the working tree, and writes nothing outside `dist/`.
# Note that `just` shows only the last comment line above a recipe in
# `--list`, so each recipe keeps its own one-line description.

# Run the frontend unit tests.
[group('check')]
[group('allow-agent')]
ui-test:
    cd "{{ ui_dir }}" && npm test

# Run Svelte and TypeScript diagnostics.
[group('check')]
[group('allow-agent')]
ui-check:
    cd "{{ ui_dir }}" && npm run check

# Build the frontend bundle.
[group('check')]
[group('allow-agent')]
ui-build:
    cd "{{ ui_dir }}" && npm run build

# Run the desktop e2e suite against a WebDriver-enabled debug build.
#
# Not part of `check`: it builds a second binary, opens a real window,
# and takes minutes. Run it when the drag surface or the sidebar markup
# changes — those are the parts no HTTP assertion can reach.
#
# The `wdio` cargo feature is what puts a W3C WebDriver server inside
# the app; without it the binary has no way in, which is the point. The
# e2e config supplies the matching capability. Both are opt-in here and
# nowhere else, so `just dogfood-build` cannot produce a remotely
# drivable app.
#
# Carries `allow-agent`, and **not** on the terms the frontend three
# carry it: this one is minutes rather than seconds, builds a second
# binary, and opens a real window. It is here for the other half of that
# group's reason — an agent without it can only report that it did not
# verify. That is not hypothetical: the `Pixels` sort axis shipped with
# five WebView assertions on the axes beside it and none of its own,
# and the agent that added them read this recipe's missing annotation as
# a deliberate refusal rather than an omission. Cost is a reason to run
# it deliberately, not a reason to leave the only surface that can check
# the grid out of reach.

# Run the desktop e2e suite in a real window (minutes; builds a binary).
[group('check')]
[group('allow-agent')]
ui-e2e: ffmpeg-sidecar
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{ ui_dir }}"
    # `tauri build`, not `cargo build`: a plain cargo build sets
    # `cfg(dev)`, and a dev-mode Tauri app loads `devUrl`
    # (localhost:5173) instead of the embedded frontend — so the window
    # comes up blank unless a Vite server happens to be running, and
    # every selector times out against nothing. `--debug` keeps the
    # debug profile, `--no-bundle` skips the .app/.dmg the suite does
    # not need. `VITE_WDIO` pulls in the frontend half of the plugin
    # pair during the beforeBuildCommand.
    VITE_WDIO=1 npx tauri build --debug --no-bundle \
        --features wdio --config src-tauri/tauri.e2e.conf.json
    npx wdio run wdio.conf.ts

# Check that JavaScriptCore agrees with the collation golden (macOS).
#
# The grid sort is a two-sided contract and its order gets frozen into
# `asset_bucket.position`, so the UI comparator and the Rust one have to
# reach the same answer. `just ui-test` checks the UI half on Node, but
# the app runs in WKWebView — a different ICU build. This recipe runs the
# same corpus through JSC, the engine that actually ships, and diffs it
# against the golden the other two consumers read. See
# `fixtures/collation/README.md`.
#
# Not part of `check`: `jsc` only exists on macOS, and a missing engine
# should not fail the standard loop.
[group('check')]
collation-jsc:
    #!/usr/bin/env bash
    set -euo pipefail
    jsc=/System/Library/Frameworks/JavaScriptCore.framework/Versions/A/Helpers/jsc
    if [ ! -x "$jsc" ]; then
        echo "jsc not found at $jsc (macOS only) — skipping" >&2
        exit 0
    fi
    cd fixtures/collation
    "$jsc" jsc-order.js | diff -u golden-icu.txt - \
        && echo "JavaScriptCore matches golden-icu.txt"
