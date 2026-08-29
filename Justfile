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
#
# (1) `cargo install cargo-aidoc`, **0.2.2 or newer**. Three things this
# workspace needs were unreleased at various points: the rustdoc JSON
# filename came from the package name, which asterism-ui's `[lib] name =
# "asterism_ui_lib"` breaks; `--title` did not exist; and the toolchain
# was the `nightly` channel rather than a pin. The note here once said
# to install from git, then said 0.2.0 was enough — it was not, and the
# version is stated rather than implied because an older one fails on an
# argument it does not have, which says nothing about why.
#
# (2) The dated nightly that tool asks for:
#
#     rustup toolchain install "$(cargo aidoc --print-required-toolchain)"
#
# Dated, because rustdoc's JSON carries a `format_version`, every
# nightly emits exactly one, and it moves whenever rustdoc's types do —
# the channel is a moving schema a pinned reader cannot follow. Asked
# for rather than written down here, because the pin belongs to the tool
# and a copy of it here would go stale on the next upgrade.
#
# This is why neither aidoc recipe *runs unconditionally* inside
# `check`: the workspace pins no toolchain (no rust-toolchain.toml) and
# a machine without this one must still be able to run the full gate.
# `aidoc-guard` is how that is reconciled — it warns instead of skipping
# silently.
#
# `--title` pins the llms.txt H1. The tool's default is the checkout
# directory's basename, which from a worktree named after its branch
# would bake the branch slug into a committed artifact — and then
# `aidoc-check` reports drift from every other checkout.
#
# `--strict` promotes the doc lints from warnings to errors, which is
# what `aidoc-check` and `aidoc-guard` have always done. Writing without
# it produced artifacts that the very next gate rejected: the lints are
# `missing-crate-doc`, `short-crate-doc`, `missing-module-doc` and
# `llms-full-too-large`, none of which regenerating can fix, so the
# difference was only ever *where* the author found out. Failing at the
# point of writing says which crate or module is missing its doc block
# while the person is still looking at it.
#
# It also matters now that CI regenerates: `aidoc-guard` steps aside
# when this recipe has already run in the same job, and it may only do
# that because this recipe applies the same bar. Drop `--strict` here
# and that skip starts hiding lint failures — see `aidoc-guard`.
#
# Run this after changing any public API or doc comment, and commit
# the diff.
aidoc:
    cargo aidoc --workspace-root "{{ project_root }}" --strict --title asterism

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

# Check the committed doc artifacts, or say out loud that nobody did.
#
# `aidoc-check` cannot be a hard step of `check` — it needs a nightly
# toolchain this workspace does not pin, and a machine without one must
# still be able to run the full gate. Leaving it out entirely is the
# other failure, and it is the one that actually happened: a change that
# deleted a crate and added another left `docs/aidoc/` describing the
# deleted one, `just check` went green, and the drift was caught by a
# human reading the diff. A gate nobody is told they skipped is not a
# gate.
#
# So: run it when it can run, and fail on drift the way it always did.
# When it cannot, print what is missing and exit 0 — the artifacts are
# unchecked, and the person running this now knows it.
[group('check')]
aidoc-guard:
    #!/usr/bin/env bash
    set -uo pipefail
    # The fifth way this cannot run, and the only one that is nobody's
    # fault: the artifacts were regenerated earlier in this same run.
    # CI does that (`.github/workflows/check.yml`) and sets this, and
    # from here the check would compare a regeneration against the
    # regeneration that produced it — it cannot report drift, and it
    # spends a second rustdoc pass over the workspace saying so. That
    # pass was 39 s of the 11 m 01 s run 31906361509, on 2026-08-15 —
    # the same run the workflow names for its other timings.
    #
    # This is only sound because `just aidoc` runs every check this
    # recipe would, and reaches each of them first:
    #
    #   - drift — impossible against a regeneration of the same tree;
    #   - target mismatch — `just aidoc` refuses to write and exits
    #     non-zero rather than retargeting the artifacts;
    #   - rustdoc format mismatch — raised before either mode branches;
    #   - the doc lints — `--strict`, which `just aidoc` carries for
    #     this reason among others. Without it there would be a real
    #     gate here and this skip would swallow it, since `--strict`
    #     runs nowhere else by default: on a developer machine the
    #     guard exits 0 unless the pinned nightly is installed.
    #
    # Said out loud rather than skipped quietly, on the same terms as
    # the four warnings below — with the difference that here the
    # artifacts *were* checked, by the tool that wrote them.
    if [ -n "${ASTERISM_AIDOC_REGENERATED:-}" ]; then
        echo "docs/aidoc/ was regenerated earlier in this run; not re-checked." >&2
        exit 0
    fi
    if ! command -v cargo-aidoc >/dev/null 2>&1; then
        echo "WARNING: docs/aidoc/ NOT CHECKED — cargo-aidoc is not installed." >&2
        echo "         cargo install cargo-aidoc" >&2
        exit 0
    fi
    # Ask the tool which toolchain it reads rather than testing for a
    # `nightly` of any date: it pins one, and having some other nightly
    # installed is not the same as having that one.
    required=$(cargo aidoc --print-required-toolchain 2>/dev/null)
    if [ -z "$required" ]; then
        echo "WARNING: docs/aidoc/ NOT CHECKED — cargo-aidoc is too old to say" >&2
        echo "         which toolchain it needs. cargo install cargo-aidoc" >&2
        exit 0
    fi
    if ! rustup toolchain list 2>/dev/null | grep -q "^${required}"; then
        echo "WARNING: docs/aidoc/ NOT CHECKED — ${required} is not installed." >&2
        echo "         rustup toolchain install ${required}" >&2
        exit 0
    fi
    # The third way this cannot run: the toolchain the tool asks for is
    # not installed, or a `--toolchain` override points at one whose
    # rustdoc JSON format it cannot read. Either way the tool says so
    # and exits 1 — a statement about the environment rather than about
    # this repository, so it belongs with the other two warnings instead
    # of turning every gate red. Drift still exits 2 and still fails.
    #
    # Since the tool pins its own nightly this should now only happen
    # when somebody has not installed it, and the message names the
    # `rustup` line that fixes that.
    output=$(cargo aidoc --workspace-root "{{ project_root }}" --check --strict --title asterism 2>&1)
    status=$?
    printf '%s\n' "$output"
    if [ "$status" -eq 0 ]; then
        exit 0
    fi
    # The fourth way this cannot run, and the first that is about the
    # machine rather than the tool. `docs/aidoc/` records the target it
    # describes (cargo-aidoc 0.3.0), and two of `asterism-infra`'s job
    # modules are behind `#[cfg(target_os = "macos")]` — from anywhere
    # else, every diff this reports is `cfg` resolution rather than
    # drift, and none of it is fixable from here. The tool says so with
    # exit 3 instead of exit 2, which is the whole reason it can be
    # told apart from the drift this recipe exists to fail on.
    #
    # CI regenerates on the recorded target before it checks
    # (`.github/workflows/check.yml`), so this branch is about the
    # machine somebody is typing on, not about the gate.
    if [ "$status" -eq 3 ]; then
        echo "WARNING: docs/aidoc/ NOT CHECKED — the artifacts describe another" >&2
        echo "         target, and CI regenerates them on it." >&2
        exit 0
    fi
    if printf '%s' "$output" | grep -q 'rustdoc format version mismatch'; then
        echo "WARNING: docs/aidoc/ NOT CHECKED — cargo-aidoc and this nightly" >&2
        echo "         disagree on the rustdoc JSON format. Update cargo-aidoc," >&2
        echo "         or pin a nightly it was built against." >&2
        exit 0
    fi
    exit "$status"

# The gates that are the same whoever runs them.
#
# Split out so that `check` and `pre-push` share one list rather than
# each carrying a copy: `check` is this plus `rust-clippy` and
# `rust-test`, `pre-push` is this plus their `-changed` counterparts. A
# gate added here is picked up by both, which a duplicated list would
# not do.
#
# What is *not* in here is exactly what scales with the size of the
# workspace rather than the size of the change. `rust-fmt-check` reads
# files and compiles nothing; `bindings-check` builds one package;
# `ui-test`, `ui-check` and `ui-build` are seconds of Node. The two
# left out — clippy and the test suite — compile every crate, and one
# of them links every test binary.
#
# `aidoc-guard` sits here rather than with those two despite doing a
# rustdoc pass over the workspace: it is not narrowable by package,
# since the artifacts it checks are one inventory of the whole tree.
#
# Every gate whose cost does not scale with the workspace.
[group('check')]
check-shared: rust-fmt-check md-check bindings-check ui-test ui-check ui-build aidoc-guard

# Run all Rust and frontend checks. The definition of green, and what
# `main` gets.
#
# This is the full-workspace shape, and a hosted runner whose load
# nobody else shares is where it belongs. It runs on every push to
# `main` — a prose-only push starts no run at all, since there would be
# nothing for the workspace to answer.
#
# A pull request gets `check-changed` instead. Both are CI's; they
# differ in what a run is asked about, not in how green is defined.
#
# A gate added here and not to `check-shared` will never run on a pull
# request, because `check-changed` below is a separate list. Add to
# `check-shared` unless the gate is genuinely `main`-only, and edit the
# two together when it is not.
#
# Run all Rust and frontend checks (the full workspace suite).
[group('check')]
check: check-shared rust-clippy rust-test

# What a pull request is asked: does what this branch changed still
# hold?
#
# The same list as `check` with the two workspace-wide gates swapped for
# their `-changed` counterparts — the substitution `pre-push` already
# makes, now made in CI too. A pull request that edits one crate stops
# paying for every crate's test binaries to be linked, and one that
# edits no crate at all links no test binary and runs no lint:
# `changed-packages` reports nothing and both gates say so and stop.
# Not "no Rust" — `check-shared` still runs `bindings-check`, which
# compiles `asterism-ui` and with it most of the workspace, and on a
# fork `aidoc-guard` still makes its own rustdoc pass. What goes away
# is the linking, which is where the load is.
#
# The sentinel is the case to watch. A change to the root manifest, the
# lockfile, the toolchain, `fixtures/` or a `scripts/` file the build
# reads is attributable to no single member, and both gates then run
# the full recipe *when `CI` is set* — because deferring to CI is not
# available to CI. Locally they still decline. Any dependency bump
# touches `Cargo.lock` and therefore takes this path.
#
# What this gives up is a regression in a crate the branch did not
# edit — a dependent that the change breaks without touching. `main`'s
# own run is where that surfaces, one merge later than before. That is
# a real delay and it is the trade: the alternative is every pull
# request linking every test binary in the workspace to find the case
# that is rare.
#
# It needs a base to compare against, which is why the workflow checks
# out full history. `changed-packages` fails loudly when it cannot find
# one rather than reporting that nothing changed.
#
# Run the checks the branch's own diff calls for (CI, pull requests).
[group('check')]
check-changed: check-shared rust-clippy-changed rust-test-changed

# Cut the worktree for an issue, and hand it a warm target directory.
#
# The two git commands are the ones the Branches section of
# CONTRIBUTING.md already prescribes. What this adds is the copy: a
# fresh worktree has no `target/`, so its first gate rebuilds the whole
# dependency graph — 21 crates' worth of work this machine may have
# done an hour ago, one directory away. Measured on `asterism-infra`
# (753 dependencies): `cargo check` took 1 min 17 s in a cold worktree
# against 39 s in a copied one.
#
# A copy and deliberately not a shared directory. Cargo treats path
# dependencies carrying the same name, version and workspace-relative
# path as the same crate even across checkouts (rust-lang/cargo#12516,
# open on 1.95), which every crate here satisfies against every other
# worktree. Point two worktrees at one target directory and a gate can
# go green against the other branch's binaries — silently, whenever its
# sources are older than that directory's last build, which is what an
# afternoon in a worktree cut this morning looks like. Copies collide
# with nothing, and they do not queue behind cargo's build lock either.
#
# What makes the copy worth making is that it is not a copy of the
# bytes, and which mechanism does that is a per-OS question. On APFS
# `cp -c` clones: 2.9 GB in under three seconds, no disk consumed until
# one side writes, and mtimes preserved — that last one is the point
# rather than a detail, since cargo's fingerprints compare them and a
# copy that reset them would rebuild everything and save nothing. On
# Linux the clone is `cp --reflink=always`, and the filesystems that
# answer to it are btrfs, bcachefs and XFS formatted with `reflink=1`.
# ext4 is not among them, which is what a stock install of most
# distributions leaves on `/`.
#
# Where Linux has no clone this hardlinks, which is the one remaining
# way to hand over a target directory without copying it. Measured on
# ext4: `cp -al` over a 111 GB target of 74,802 files took 1.4 seconds
# and consumed no disk, against 6.3 minutes and 111 GB for the byte
# copy of the same tree. The byte copy is not just slower — two
# worktrees' worth of it does not fit beside a checkout that already
# holds one.
#
# The part of it that is a real copy — see below — runs in the
# background, and the recipe returns in about two seconds. Measured on
# this checkout: 45 seconds for the copy with the tree in page cache,
# and six minutes reading it cold.
#
# A hardlink shares the inode, so a write through one path is a write
# to the other, and only one part of a target directory is safe to
# share on those terms: the large artifacts under `deps/`. Cargo names
# those by a hash of what went into them and replaces them by unlinking
# its own copy first, so a build here leaves the other side's bytes
# alone. Everything else has a writer that opens the existing file:
# rustc truncates its dep-info in place, cargo rewrites its own
# fingerprints and `.rustc_info.json`, build scripts re-run into an
# `OUT_DIR` cargo does not clear first, rustdoc overwrites its JSON,
# and `.cargo-lock` is the inode two checkouts would queue on. So the
# split is `deps/` at a megabyte and up — 2,307 files, 85.56 GiB —
# shared, and the remaining 33,368 files of 10.44 GiB copied. Nothing
# new falls on the sharing side by accident: it has to be large and it
# has to be in `deps/`.
#
# `incremental/` is dropped rather than either, since cargo
# regenerates it and it is 18 GB of the 111. Staged that way, a
# worktree cut here builds 4 to 16 crates where a cold one builds the
# 753-crate graph. Linux only, because Linux is where the fallback was
# needed.
#
# The filesystem is asked before the copy rather than inferred from the
# exit status afterwards, and the two need that for opposite reasons.
# `cp -c` does not fail where clonefile(2) is unavailable: it "will
# fallback to using copyfile(2) instead to ensure the copy still
# succeeds" (man cp). That fallback is a real multi-gigabyte byte copy
# — the outcome this recipe exists to avoid — and it would report
# success while doing it. GNU cp is the mirror image: with
# `--reflink=always` it does not fall back, and instead will "report
# the failure for each file and exit with a failure status" (info
# coreutils). The status is honest, but `-a` implies `-R` and cp keeps
# going after each failure, so the honesty arrives one line per file.
# Measured on ext4: a tree of eight files produced eight error lines
# and a complete directory skeleton holding none of them, and the
# `target/` on that machine held 74,802 files.
#
# Not covered: a build running in this checkout while the copy reads
# its `target/`. Cargo's own lock (`target/debug/.cargo-lock`) answers
# that question and this recipe does not consult it. A torn snapshot
# does not fail the copy; it surfaces later as an artifact sitting
# behind a fingerprint that says fresh. Cut worktrees between builds.
#
# Carries `allow-agent`: cutting the worktree is the first thing an
# agent does with an issue. It is not confined the way most of that
# group is — `git fetch` reaches the network and moves remote-tracking
# refs, and `git worktree add -b` creates a branch — so it sits here on
# `ui-e2e`'s half of the group's reasoning rather than the format
# checks': an agent without it cannot start.

# Cut a worktree for an issue, with a target directory cloned into it.
#
# `[positional-arguments]` rather than `{{ }}` inside the body, because
# `{{ slug }}` is a textual substitution: `just` writes the argument
# into the script and bash parses the result, so a slug carrying
# `$(...)` runs before any guard below can look at it — including at
# the `case` that exists to reject it, which is itself a substitution
# site. Measured: `worktree-new feat 'x$(echo hi >&2)y'` printed `hi`
# three times, passed the guard, and made a worktree at a path the
# caller never wrote. As `$1` and `$2` the same argument is inert data
# the guard can actually test.
#
# Nothing here deletes anything it did not just write. The removals
# are the clone probe's pair of files, and the directories inside the
# staged link tree whose links have to be broken — both of them a few
# lines after the same block makes them, and the second kind holding
# nothing but links made moments earlier, so unlinking them leaves the
# main checkout's bytes where they were. Every other path built below
# is somewhere this recipe creates, so the worst a wrong one can do is
# put a directory in an odd place, which is a thing to move by hand,
# not a thing to lose work to. What it creates and where is in the
# body.
[group('worktree')]
[group('allow-agent')]
[positional-arguments]
worktree-new type slug:
    #!/usr/bin/env bash
    set -euo pipefail
    kind="$1"
    slug="$2"
    # A worktree cannot cut another one. Nothing stops git from nesting
    # `.worktrees/` inside a worktree, which is the trap: it succeeds,
    # and what it hands back is a copy of a copy on a branch nobody
    # meant to stack.
    if [ "$(git rev-parse --absolute-git-dir)" \
       != "$(git rev-parse --path-format=absolute --git-common-dir)" ]; then
        echo "worktree-new runs in the main checkout, not inside a worktree." >&2
        exit 1
    fi
    # `slug` becomes a directory name below, and both halves become a
    # branch name, so each has to be one ordinary segment. Stated as
    # what is allowed rather than as a list of what is not: the
    # characters a directory and a branch can both carry without
    # quoting, and nothing else.
    case "$slug" in
        ""|*[!A-Za-z0-9._-]*|.*|*..*)
            echo "slug must be one segment of [A-Za-z0-9._-], not starting" >&2
            echo "with a dot and containing no '..': got '$slug'" >&2
            exit 1
            ;;
    esac
    case "$kind" in
        ""|*[!A-Za-z0-9._-]*|.*)
            echo "type must be one segment of [A-Za-z0-9._-]: got '$kind'" >&2
            echo "CONTRIBUTING.md names the ones in use: ci, fix, feat, docs." >&2
            exit 1
            ;;
    esac
    git fetch origin
    # `branch-check`'s third assertion, hoisted ahead of the worktree.
    # It fails on the state of local `main` rather than on anything
    # about the branch being cut, and failing it after the worktree and
    # its copy exist would leave both behind with this command no
    # longer re-runnable — `git worktree add` refuses a branch and a
    # directory that are already there.
    if ! git merge-base --is-ancestor main origin/main; then
        echo "local main carries commits origin/main does not have:" >&2
        echo "reset it to origin/main before cutting branches." >&2
        exit 1
    fi
    dest="{{ project_root }}/.worktrees/$slug"
    src="{{ project_root }}/target"
    git worktree add "$dest" -b "$kind/$slug" origin/main
    # How this worktree gets its target directory. `clone_flag` is the
    # cp flag that clones where the filesystem clones; `stage_mode`
    # names the fallback where it does not. Both empty means neither is
    # available and the reason has already been said. Which one applies
    # is the per-OS half; the two answers ahead of it hold on any OS.
    clone_flag=""
    stage_mode=""
    if [ ! -d "$src" ]; then
        echo "NOTE: no target/ in this checkout to copy; worktree starts cold." >&2
    # Both sides, not just the source. A clone lands within one
    # filesystem and not across two, and the case that reaches this is
    # not obvious from the paths: `.worktrees/` is free to be a symlink
    # or a mount of its own, and then the copy silently becomes the
    # multi-gigabyte one this whole branch exists to avoid.
    elif [ "$(df -P "$src" | awk 'NR == 2 { print $1 }')" \
        != "$(df -P "$dest" | awk 'NR == 2 { print $1 }')" ]; then
        echo "NOTE: target/ and the new worktree are on different volumes, so" >&2
        echo "      there is no clone to make between them. Worktree starts" >&2
        echo "      cold." >&2
    else
        case "$(uname -s)" in
            Darwin)
                if mount | grep -qE "^$(df -P "$src" | awk 'NR == 2 { print $1 }') on .*\(apfs"; then
                    clone_flag="-c"
                else
                    echo "NOTE: target/ is not on an APFS volume, so copying it would be a" >&2
                    echo "      real multi-gigabyte copy rather than a clone, and would" >&2
                    echo "      cost more than the build it saves. Worktree starts cold." >&2
                fi
                ;;
            Linux)
                # Asked of the filesystem rather than of its name. btrfs
                # and bcachefs always answer yes and ext4 always no, but
                # XFS answers by how it was made — `reflink=1`, mkfs's
                # default only since xfsprogs 5.1 — and a container
                # layer or a network mount can differ from whatever the
                # mount table suggests. One clone of one file settles
                # it. 8 KiB rather than an empty file because btrfs
                # keeps a small enough file inline in its metadata,
                # where cloning is a different question from the one
                # being asked.
                probe=""
                if probe="$(mktemp -d "$dest/.reflink-probe.XXXXXX")" \
                    && head -c 8192 /dev/zero > "$probe/a" \
                    && cp --reflink=always "$probe/a" "$probe/b" 2>/dev/null; then
                    clone_flag="--reflink=always"
                else
                    # Three failures share this branch — the directory,
                    # the file, the clone — and all three mean the same
                    # thing here, so none of them is named as a cause
                    # that was not measured. Only the clone's own error
                    # is silenced; the other two print above.
                    stage_mode="hardlink"
                fi
                # The two files the lines above wrote and the directory
                # holding them, and nothing else: `rmdir` refuses a
                # directory with anything else in it. Neither removal
                # can take the recipe down — by this line the worktree
                # exists, and a second run of this command could not
                # make it again — so a probe that survives is announced
                # instead, and announced with what it costs: it is
                # untracked, and the `-changed` gates refuse a tree with
                # anything untracked in it.
                if [ -n "$probe" ]; then
                    rm -f "$probe/a" "$probe/b" 2>/dev/null || :
                    rmdir "$probe" 2>/dev/null || {
                        echo "NOTE: left the clone probe at $probe — remove it" >&2
                        echo "      before the -changed gates will answer." >&2
                    }
                fi
                ;;
            *)
                echo "NOTE: this recipe knows the clone for macOS and for Linux," >&2
                echo "      and $(uname -s) is neither, so copying target/ would" >&2
                echo "      be a real multi-gigabyte copy rather than a clone." >&2
                echo "      Worktree starts cold." >&2
                ;;
        esac
    fi
    # Staged under a name cargo does not read, and renamed into place
    # only once the copy says it finished. `-a` implies `-R`, and in -R
    # mode cp "will continue copying even if errors are detected" (man
    # cp), so a failure partway through leaves a tree with files missing
    # while the fingerprints beside them say fresh — and an interrupt
    # leaves the same thing with nothing to announce it. Under the
    # staged name neither is a `target/` at all, so cargo never reads
    # one, and a half-made one is inert rather than wrong.
    #
    # Inside `workspace/`, which `.gitignore` covers, because a staged
    # tree at the worktree's root would be untracked — and the
    # `-changed` gates refuse a tree with anything untracked in it,
    # which would make an unfinished copy block the branch's own gates.
    mkdir -p "$dest/workspace"
    staged="$dest/workspace/target.partial"
    if [ -n "$clone_flag" ]; then
        # Seconds, so it happens here rather than behind the prompt.
        #
        # A failed one is left where it is. It affects nothing, and
        # clearing disk is not worth a recipe that deletes.
        if cp -a "$clone_flag" "$src" "$staged" \
            && mv "$staged" "$dest/target"; then
            echo "cloned target/ into the worktree"
        else
            echo "NOTE: copying target/ did not finish — cp's own error, if" >&2
            echo "      any, is above. The worktree starts cold. An" >&2
            echo "      incomplete copy may be left at $staged" >&2
        fi
    elif [ "$stage_mode" = hardlink ]; then
        # In the background, because this half is minutes rather than
        # seconds — 10 GB of small files, at whatever a shared disk
        # gives — and nothing reads `target/` until something compiles.
        # What follows cutting a worktree is reading the issue and the
        # code and settling on an approach, so the copy runs through
        # that and is there before the first build asks for it. Cargo
        # cannot see a half-made tree under the staged name, so the
        # window costs nothing but a cold build to anything that does
        # compile inside it.
        #
        # SIGHUP ignored so that closing the terminal that ran this does
        # not leave the copy half-done.
        log="$dest/workspace/target-staging.log"
        {
        trap '' HUP
        broke=0
        if ! cp -al "$src" "$staged"; then
            broke=1
        else
            # Dropped rather than copied: cargo regenerates it, and it
            # is the one large thing here that nothing needs carried
            # over — 18 GB of the 111.
            find "$staged" -type d -name incremental -prune -exec rm -rf {} + || broke=1
            # Then everything that is not a large artifact under
            # `deps/` gets a copy of its own. `--parents` keeps each
            # file's path under the staged root, so this is one pass
            # over the tree, and `--remove-destination` unlinks the
            # shared inode before writing rather than writing through
            # it — which is the entire point, and is not cp's default.
            ( cd "$src" \
                && find . -type d -name incremental -prune -o \
                          -type f -path '*/deps/*' ! -size -1048576c -o \
                          -type f -print0 \
                   | xargs -0 -r cp -a --remove-destination --parents \
                           --target-directory="$staged" ) \
                || broke=1
        fi
        if [ "$broke" != 0 ]; then
            echo "linking target/ did not finish — the error, if any, is above."
            echo "the worktree stays cold; what was staged is at $staged"
        elif [ -e "$dest/target" ]; then
            # Something compiled before this landed. That build's
            # `target/` is the one cargo has been writing fingerprints
            # into, and replacing it underneath a running or finished
            # build is how a tree ends up half from one place and half
            # from another.
            echo "the worktree already has a target/ — a build got there first,"
            echo "so this staging stands down. What it staged is at $staged"
        elif mv "$staged" "$dest/target"; then
            echo "target/ is in place: the large artifacts under deps/ shared"
            echo "with the main checkout, everything else copied"
        else
            echo "could not move the staged tree into place; it is at $staged"
        fi
        } > "$log" 2>&1 &
        echo "staging target/ in the background — until it lands this worktree"
        echo "builds cold, and $log says when it is done."
    fi
    cd "$dest" && just branch-check
    echo "worktree ready: $dest (branch $kind/$slug)"

# Fail unless the current branch is a worktree branch cut from
# origin/main. The incident this exists for (2026-08-15): a branch cut
# from a local main that had silently diverged from origin/main carried
# the entire pre-publication history into a push. Three facts, all
# mechanical: not on main, origin/main is an ancestor of HEAD, and
# local main holds nothing origin/main lacks (ancestry alone passes a
# main that is merely *ahead* — unpushed commits ride along exactly as
# in the incident). A stale remote-tracking ref weakens the last two;
# the recipe stays offline on purpose, so fetch before relying on it.
# Deliberately not a dependency of `check` — a human validating main
# after a merge runs `check` on main legitimately; this gate is for the
# start of work, not the end.
#
# Carries `allow-agent` on the same terms as the format checks: it
# reads refs and writes nothing, and an agent that cannot run it can
# only guess at its base.
[group('check')]
[group('allow-agent')]
branch-check:
    @test "$(git branch --show-current)" != "main" || { echo "on main: cut a worktree branch first (see AGENTS.md)"; exit 1; }
    @git merge-base --is-ancestor origin/main HEAD || { echo "HEAD does not descend from origin/main: wrong base — rebuild the branch from origin/main"; exit 1; }
    @git merge-base --is-ancestor main origin/main || { echo "local main carries commits origin/main does not have: reset it to origin/main before cutting branches"; exit 1; }

# Check commit message bodies against the 72 columns CONTRIBUTING asks
# for. `scripts/check-commit-msg.py` carries the reasoning, including
# why the count is Python's rather than a shell tool's.
#
# Takes a file, a revision range, or both — the file for a message
# written but not yet committed, the range for what a branch already
# carries. `pre-push` calls it with `origin/main..HEAD`.

# Check that commit message bodies wrap at 72 columns.
[group('check')]
[group('allow-agent')]
commit-msg-check *args:
    python3 "{{ project_root }}/scripts/check-commit-msg.py" {{ args }}

# The last gate before a branch is handed over, and the agent that built
# the branch is the one that runs it. It writes to nothing remote — so
# being denied `git push` is no reason to skip it. `git fetch origin` belongs immediately before it: `branch-check`
# and `changed-packages`, which the two narrow gates call, both read
# `origin/main` offline.
#
# It is `branch-check` plus `check-shared` plus `rust-clippy-changed`
# and `rust-test-changed`, and those two substitutions are the point.
# It used to be `branch-check` plus `check`, which reached `rust-test`
# and linked all 21 crates' test binaries — one linker process each,
# gigabytes resident each — and `rust-clippy`, which compiled every
# target in every crate, on whoever's machine the branch happened to be
# built on. On 2026-08-15 that machine was shared, and the branch under
# test had changed a workflow file and two comments.
#
# Neither is a weaker gate for running in CI; they are gates run where
# the load belongs. What is lost locally is the crates a change did not
# edit, and CI reports those on the same push.
#
# 2026-08-15 (the hand-over of the #39 branch, not the `branch-check`
# divergence dated above): this comment used to say a human runs it and
# that agents never reach it. An agent duly put `just pre-push` into
# the command block it handed over, instead of running it and
# reporting what it found.
#
# The two cheap assertions come first and in this order on purpose:
# both answer questions no build can change, and a branch that fails
# either fails it just as surely after twenty minutes of compiling.
#
# A branch that edits nothing but prose stops after those two. CI
# already works that way — `paths-ignore` in `.github/workflows/check.yml`
# starts no run for a push that touches only those files — so a local
# run of the build gates over such a branch is minutes of a shared
# machine spent reproducing a verdict nobody asked for and nothing
# reads. The list is read from the workflow rather than copied here, so
# the two cannot drift; the workflow's own comment is where the
# reasoning for each entry lives, including which files are deliberately
# absent from it.
#
# What a prose branch is left with is the two assertions above and a
# reading of the diff — the three reviews, which the message this
# recipe prints names. None is a recipe, and none is something
# `pre-push` can stand in for.
#
# Run every gate over the tree being handed over.
[group('check')]
pre-push: branch-check (commit-msg-check "--range" "origin/main..HEAD")
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{ project_root }}"
    changed=$(git diff --name-only origin/main...HEAD)
    if [ -z "$changed" ]; then
        echo "no commits on this branch that origin/main does not have." >&2
        exit 1
    fi
    # The workflow's patterns are glob, and `**` there means what `*`
    # means to bash once the path separator is not special to it.
    #
    # Read into the array a line at a time rather than with `mapfile`,
    # which is a bash 4 builtin. macOS ships bash 3.2 and always will —
    # Apple stopped at the last GPLv2 release — so `mapfile` here made
    # this recipe run on the CI runner (which has a newer bash from
    # homebrew on its PATH) and fail on a stock Mac, where it is a
    # `command not found` after the two assertions above have already
    # passed. The loop below is what the rest of this recipe already
    # uses.
    ignored=()
    while IFS= read -r pattern; do
        ignored+=("$pattern")
    done < <(
        awk '/paths-ignore:/ { f = 1; next }
             f && /^[[:space:]]*-[[:space:]]/ {
                 gsub(/^[[:space:]]*-[[:space:]]*/, "")
                 gsub(/^.|.$/, "")
                 print
                 next
             }
             f { exit }' .github/workflows/check.yml
    )
    if [ "${#ignored[@]}" -eq 0 ]; then
        echo "could not read paths-ignore from .github/workflows/check.yml;" >&2
        echo "running every gate rather than guessing." >&2
        prose_only=false
    else
        prose_only=true
        while IFS= read -r path; do
            matched=false
            for pattern in "${ignored[@]}"; do
                # shellcheck disable=SC2053
                if [[ "$path" == ${pattern/\*\*/\*} ]]; then
                    matched=true
                    break
                fi
            done
            if [ "$matched" = false ]; then
                prose_only=false
                break
            fi
        done <<< "$changed"
    fi
    if [ "$prose_only" = true ]; then
        # Except the one gate that is about prose. `check-shared` carries
        # it for every other branch; a branch that edits nothing but
        # markdown is the last one that should skip it.
        just md-check
        echo
        echo "This branch edits only files the CI workflow's paths-ignore covers:"
        printf '  %s\n' $changed
        echo "No build gate starts for them in CI, so none runs here either. Read"
        echo "the diff instead — pub-checker for the disclosure policy,"
        echo "doc-reviewer for the prose, reviewer for the rest."
        exit 0
    fi
    just check-shared rust-clippy-changed rust-test-changed

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

# Run the tests of named packages only — the narrow half of the pair.
#
# `rust-test` links the whole workspace, and linking is where the load
# is: one linker process per test binary, gigabytes of resident memory
# each, as many at once as `jobs` allows. On a shared or memory-tight
# machine that is enough to push the box into swap and take every other
# session on it down with the run. Naming the crates a change touches
# costs a fraction of that.
#
#   just rust-test-pkg asterism-core asterism-server
#
# This is not a weaker gate, it is a narrower one, and it is the one to
# reach for while iterating: the full suite runs on every push to
# `main` that changes code, so opening a PR does not wait on a workspace
# run happening here first — and neither does the PR's own CI run, which
# asks the same narrow question this recipe does. (A push that touches
# nothing but prose starts no
# run at all — see the workflow's `paths-ignore`.)
# `rust-test-changed` picks the arguments for you from the branch diff,
# and is what `pre-push` runs; reach for this one when you already know
# which crates you care about.
#
# Keeps `--no-fail-fast` and its own exit status per package for the
# reason the full recipe does: one crate failing must not hide the rest.
# It keeps no log. It counts binaries only when `CI` is set: that check
# exists to make a run auditable, a two-crate run is read in the
# terminal by the person who started it, and a runner has no such
# person.
#
# Run the tests of the named packages (the narrow alternative to rust-test).
[group('check')]
rust-test-pkg +packages:
    #!/usr/bin/env bash
    set -uo pipefail
    cd "{{ project_root }}"
    export CARGO_TERM_COLOR=never
    status=0
    launched=0
    reported=0
    for pkg in {{ packages }}; do
        echo
        echo "=== $pkg ==="
        # Counted, not just streamed, when nobody is watching the
        # stream. `rust-test` checks that every launched binary reported
        # a result because one killed mid-run prints no `test result:`
        # line and its passes vanish from the sum — the shape of the
        # 2026-07-30 observation, which is as much as anyone established
        # about it. This recipe left that out on the grounds that a
        # two-crate run is read in the terminal, which was true while a
        # person was the only caller. CI runs this now, and a runner has
        # no terminal anyone reads.
        if [ -n "${CI:-}" ]; then
            out=$(cargo test -p "$pkg" --no-fail-fast 2>&1) || status=1
            printf '%s\n' "$out"
            launched=$((launched + $(printf '%s\n' "$out" | grep -cE '^ +(Running|Doc-tests)')))
            reported=$((reported + $(printf '%s\n' "$out" | grep -cE '^test result:')))
        else
            cargo test -p "$pkg" --no-fail-fast || status=1
        fi
    done
    if [ -n "${CI:-}" ]; then
        echo
        echo "binaries: $launched launched / $reported reported"
        if [ "$launched" -ne "$reported" ]; then
            echo
            echo "MISSING: $((launched - reported)) binary/binaries never reported a" >&2
            echo "result. Their tests are absent from the counts above — the totals" >&2
            echo "are a floor, not a tally." >&2
            status=1
        fi
    fi
    if [ "$status" -ne 0 ]; then
        echo
        echo "One or more packages failed; the output above is the whole run." >&2
    fi
    exit "$status"

# Run one package's tests, with everything after the package name handed
# to `cargo test` verbatim.
#
# The edit loop's recipe. `rust-test-pkg` is still a whole crate, and a
# crate is not a small unit here: `asterism-core` builds its whole unit
# suite and `asterism-server` links every one of its integration
# binaries, so "narrow" at package granularity is still minutes for a
# one-line change. Nothing
# above this line could say *which* test, and that is what makes the
# gates unusable while actually writing code.
#
# Everything after the package is cargo's, not this file's, so the whole
# of `cargo test`'s selection vocabulary is reachable without this
# recipe learning any of it:
#
#   just rust-test-one asterism-core edge          # names matching `edge`
#   just rust-test-one asterism-core --lib         # unit tests only
#   just rust-test-one asterism-server --test dispatch_copy_fold_e2e
#   just rust-test-one asterism-core edge -- --nocapture
#
# `--lib` is the one worth knowing: it skips the integration binaries,
# which is where the link time is.
#
# No `--no-fail-fast` here, deliberately. The gates pass it so one
# crate's failure cannot hide another's; while iterating the first
# failure is the answer and waiting for the rest is waste.
#
# This is not a gate and does not stand in for one. It answers about
# whatever you named and nothing else — `pre-push` still runs
# `rust-test-changed`.
#
# Run one package's tests, passing the rest to cargo (the edit loop's recipe).
[group('check')]
[group('allow-agent')]
rust-test-one pkg *args:
    #!/usr/bin/env bash
    set -uo pipefail
    cd "{{ project_root }}"
    export CARGO_TERM_COLOR=never
    cargo test -p "{{ pkg }}" {{ args }}

# Print the workspace members this branch touched, one per line.
#
# The shared half of `rust-test-changed` and `rust-clippy-changed`, and
# the reason both exist: `pre-push` used to reach `rust-test` and
# `rust-clippy` through `check`, so the gate before a hand-over linked
# all 21 crates' test binaries *and* compiled every target in the
# workspace, on whatever machine the branch was built on. On 2026-08-15
# that was a shared box, and a branch whose entire diff was a workflow
# file and two comments paid for both. Neither is the wrong thing to
# run — they are the wrong thing to run *here*. CI runs them on every
# push, on a runner nobody else is sitting on.
#
# What counts as touched: the paths the commits on this branch changed
# against `origin/main`, mapped to the workspace member whose directory
# contains them. The working tree is not consulted — CI asks this about
# a commit, and an answer that moves with uncommitted state is an
# answer local and CI disagree on.
#
# Prints the literal `--workspace` instead of a list when the change
# reaches the root manifest, the lockfile or the toolchain. Every crate
# is in scope then, so there is no narrow run to make, and the callers
# say so rather than quietly starting the run they exist to avoid. No
# member is named `--workspace`, so the sentinel cannot collide with a
# real answer.
#
# Empty output means no member changed, which is a normal result and
# not an error — a documentation or CI-only branch reaches it.
#
# `origin/main` is read offline, so its freshness is the last `git
# fetch origin` — the same assumption `branch-check` states, and the
# reason CONTRIBUTING puts the fetch immediately before `pre-push`.
#
# Print the workspace members this branch touched.
[group('check')]
[group('allow-agent')]
changed-packages:
    #!/usr/bin/env bash
    set -uo pipefail
    cd "{{ project_root }}"

    # Refuse to answer rather than answer "nothing" when the base is
    # not there. Everything below reads as "no package changed" if
    # `git diff` cannot resolve `origin/main` — it writes to stderr and
    # produces no paths — and on a clean tree that is indistinguishable
    # from a branch that touched no crate. A caller acting on that skips
    # every test and calls itself green.
    #
    # It is reachable: a shallow clone has no `origin/main`, which is
    # what `actions/checkout` produces by default, and CI is now a
    # caller. The workflow asks for full history for this reason;
    # this is what says so when it does not.
    if ! git rev-parse --verify --quiet origin/main >/dev/null; then
        echo "cannot resolve origin/main, so which packages changed is unknown." >&2
        echo "Run 'git fetch origin', and in CI check out with fetch-depth: 0." >&2
        exit 1
    fi
    if ! git merge-base origin/main HEAD >/dev/null 2>&1; then
        echo "origin/main and HEAD share no history — a shallow clone cannot" >&2
        echo "answer which packages changed. Check out with fetch-depth: 0." >&2
        exit 1
    fi
    # A shallow clone can have `origin/main` and still be missing the
    # merge base's ancestors, in which case the diff is against the
    # wrong point and reports too little. Refused rather than trusted:
    # the two guards above ask whether the base is *there*, and this one
    # asks whether the history behind it is.
    if [ "$(git rev-parse --is-shallow-repository)" = "true" ]; then
        echo "this is a shallow clone, so the merge base with origin/main may" >&2
        echo "not be present and the diff would report too few packages." >&2
        echo "Check out with fetch-depth: 0." >&2
        exit 1
    fi

    # The commits this branch carries, against the merge base. Nothing
    # else: what CI asks this recipe is a question about a commit, and
    # the working tree is not part of the answer there — the checkout
    # is clean and always will be.
    #
    # It used to union in `git status --porcelain`, and that made the
    # verdict depend on state no commit records. An untracked file
    # under one of the sentinel paths below flipped a branch carrying
    # no commits at all to `--workspace` (2026-08-17, this branch and
    # its own new script). Local and CI then answer differently about
    # the same commit, which is the one property a pre-push gate cannot
    # have.
    #
    # What that gives up is the edit loop, and it is given up loudly. A
    # dirty tree is refused rather than answered narrowly: an edit not
    # yet committed maps to no member, and "no member changed" reaching
    # a caller as exit 0 is a green report from a suite that never ran —
    # the worse of the two mistakes, by the same reasoning the sentinel
    # below is written around. Commit first, which is the order
    # `pre-push` imposes anyway, or run
    # `just rust-test-one <crate> <filter>` while editing.
    #
    # Not refused under `CI`, where the checkout is clean by
    # construction and any dirt is something a step in the same job
    # produced. Failing there would be a new way for CI to break in the
    # place that must not, and the branch has commits to answer for in
    # any case.
    changed=$(git diff --name-only origin/main...HEAD | sort -u)

    dirty=$(git status --porcelain)
    if [ -n "$dirty" ]; then
        if [ -z "${CI:-}" ]; then
            echo "the working tree is dirty, so this cannot answer for it —" >&2
            echo "it reports the commits on this branch and nothing else." >&2
            echo "Commit, or run 'just rust-test-one <crate> <filter>' while" >&2
            echo "editing:" >&2
            printf '%s\n' "$dirty" | sed 's/^/  /' >&2
            exit 1
        fi
        echo "NOTE: uncommitted changes in a CI checkout are not attributed." >&2
    fi

    if [ -z "$changed" ]; then
        exit 0
    fi

    # Paths under a sentinel directory that no part of the build reads.
    #
    # Named one at a time, and the default stays workspace-wide,
    # because the two mistakes are not the same size: a build-feeding
    # script left off this list costs a run that was too big, and one
    # wrongly on it costs a green report from a suite that never ran.
    #
    # `check-commit-msg.py` reads commit messages. No crate compiles it,
    # no test invokes it, and no fixture comes out of it — it is on the
    # `scripts/` path and nothing else, which was enough to make a
    # change to it compile all 21 crates and link every test binary.
    attributable=$(
        printf '%s\n' "$changed" \
            | grep -vxF 'scripts/check-commit-msg.py' || true
    )

    # Said where it is applied. An exemption nobody sees reads as
    # coverage — the same reason `check-commit-msg.py` announces the
    # commits it skips.
    exempt=$(printf '%s\n' "$changed" | grep -xF 'scripts/check-commit-msg.py' || true)
    if [ -n "$exempt" ]; then
        echo "Exempt from the workspace-wide sentinel (the build reads none" >&2
        echo "of these):" >&2
        printf '%s\n' "$exempt" | sed 's/^/  /' >&2
    fi

    if [ -z "$attributable" ]; then
        exit 0
    fi

    # A change to any of these is not attributable to one member.
    #
    # `fixtures/` and `scripts/` are here because the mapping below —
    # "the member whose directory contains the path" — is a proxy for
    # "which crates could this break", and these two are where the proxy
    # is wrong. `asterism-core`'s collation tests read
    # `fixtures/collation/`, and `asterism-infra`'s chapter-scan tests
    # need what `scripts/gen-test-fixtures.py` produces. Neither lives
    # under a member, so without this line a branch editing the corpus
    # would run nothing that reads it.
    if printf '%s\n' "$attributable" | grep -qE '^(Cargo\.(toml|lock)|rust-toolchain(\.toml)?|\.cargo/|fixtures/|scripts/)'; then
        echo "Workspace-wide change (manifest, lockfile or toolchain):" >&2
        printf '%s\n' "$attributable" | grep -E '^(Cargo\.(toml|lock)|rust-toolchain(\.toml)?|\.cargo/|fixtures/|scripts/)' | sed 's/^/  /' >&2
        echo "--workspace"
        exit 0
    fi

    # Member directory -> package name, read from each manifest rather
    # than assumed from the directory: they agree today, and a recipe
    # that depends on them agreeing breaks silently the day one is
    # renamed.
    members=$(
        awk '/^members *= *\[/ {inside=1; next} inside && /^\]/ {exit} inside' Cargo.toml \
            | tr -d ' ",'
    )
    # An empty list is not an answer. This parse wants one member per
    # line, so a `members = ["crates/a", "crates/b"]` written on one
    # line yields nothing — and every path would then map to no member,
    # which reads as "no crate changed" and would skip every test from
    # then on, silently and forever.
    if [ -z "$(printf '%s' "$members" | tr -d '[:space:]')" ]; then
        echo "no workspace members parsed out of Cargo.toml, so no path can be" >&2
        echo "attributed. Expected one member per line under 'members = ['." >&2
        exit 1
    fi

    packages=""
    for dir in $members; do
        [ -f "$dir/Cargo.toml" ] || continue
        name=$(
            awk '/^\[package\]/ {inside=1; next}
                 /^\[/ {inside=0}
                 inside && /^name *=/ {gsub(/^name *= *"|"$/, ""); print; exit}' \
                "$dir/Cargo.toml"
        )
        [ -n "$name" ] || continue
        # Each member claims its own directory and nothing above it.
        # `crates/asterism-ui` is not itself a member — only
        # `crates/asterism-ui/src-tauri` is — so the Svelte sources
        # beside it match nothing here, which is right: `ui-test` and
        # `ui-check` are what cover them.
        if printf '%s\n' "$attributable" | grep -q "^$dir/"; then
            packages="$packages $name"
        fi
    done

    if [ -z "${packages// /}" ]; then
        echo "No workspace member changed. Changed paths:" >&2
        printf '%s\n' "$attributable" | sed 's/^/  /' >&2
        exit 0
    fi

    printf '%s\n' $packages | sort -u

# Test the packages this branch touched (pre-push's narrow suite).
#
# One limit, stated because a narrower gate that reads as a full one is
# worse than no gate: it names packages a change *edited*, not packages
# that depend on them. Editing `asterism-core` does not test
# `asterism-server` here, and it is `main`'s own run that catches what
# that misses — a pull request's run asks this same narrow question.
[group('check')]
[group('allow-agent')]
rust-test-changed:
    #!/usr/bin/env bash
    set -uo pipefail
    cd "{{ project_root }}"
    packages=$(just changed-packages) || exit 1
    if [ "$packages" = "--workspace" ]; then
        # Every crate is in scope, so there is no narrow run to make —
        # and what to do about that depends entirely on who is asking.
        #
        # Locally the answer is to decline: the workspace suite is what
        # this recipe exists to keep off a developer's machine, and CI
        # will run it. In CI that same sentence would be a lie about the
        # process saying it, and the cost of the lie is the largest
        # blast radius there is. `Cargo.lock` is inside the pattern that
        # produces this sentinel, so *any* dependency bump takes this
        # branch — and a bump that skips every test and reports green is
        # the worst thing this file could do.
        if [ -n "${CI:-}" ]; then
            echo "Every crate is in scope; running the workspace suite."
            just rust-test
            exit "$?"
        fi
        echo "Every crate is in scope, so there is no narrow suite to run."
        echo "CI runs the workspace suite on this push; it is not run here."
        exit 0
    fi
    if [ -z "$packages" ]; then
        echo "No workspace member changed; no Rust test to run."
        exit 0
    fi
    # Members whose test binary would link the world to run nothing.
    #
    # `changed-packages` maps paths to members, and a member with no
    # tests maps just as readily as one with hundreds. `asterism-ui` is
    # the case that forced this: its Rust side is DI wiring, its actual
    # tests are the vitest suite `ui-test` runs in `check-shared`, and
    # `cargo test -p asterism-ui` still links the Tauri stack on top of
    # `asterism-core` and `asterism-infra` before running none. On the
    # branch that added this, seven lines of import churn in that crate
    # cost more wall clock than every other package it touched together.
    #
    # Named, not inferred — the call `changed-packages` already makes
    # for `scripts/check-commit-msg.py`, for the same reason. Deciding
    # this by pattern-matching sources reads "no match" and "could not
    # look" as one answer, and has to keep up with every spelling of a
    # test attribute: this tree writes
    # `#[tokio::test(flavor = "multi_thread")]` throughout, which the
    # obvious pattern misses. Both mistakes point the expensive way — a
    # suite that never ran, reported green.
    #
    #   asterism-importer  One `main.rs` of clap subcommands over the
    #                      importer adapters, each of which carries its
    #                      own tests. Zero test attributes.
    #
    # A member here that gains a test must come off this list. Nothing
    # local catches a stale entry — `main`'s workspace run is what does,
    # which is the same net every other narrowing in this file relies
    # on.
    #
    # `asterism-ui` came off in #175, two changes after it should have.
    # #159 moved `mutation_surface` into that crate precisely so the
    # guard would "run in the same pull request that moves its subject",
    # and left this list saying the crate had zero test attributes — so
    # the guard ran nowhere but `main`, which is the arrangement the move
    # was undoing. `export_parity` arrived beside it and would have
    # inherited the same silence. Both read a source file and link
    # nothing of the crate they sit in, but `cargo test -p asterism-ui`
    # links the Tauri stack regardless; that cost is the price of the
    # gate answering, and it is paid only on a branch that touched this
    # crate.
    testless="asterism-importer"
    run=""
    skipped=""
    for pkg in $packages; do
        case " $testless " in
            *" $pkg "*) skipped="$skipped $pkg" ;;
            *) run="$run $pkg" ;;
        esac
    done
    # Printed, never silent. A gate that quietly tests less than it says
    # is the failure this whole family of recipes exists to avoid.
    if [ -n "$skipped" ]; then
        echo "Carries no Rust test, so no test binary is built for it:$skipped"
    fi
    if [ -z "$run" ]; then
        echo "That is every member this branch touched; no Rust test to run."
        exit 0
    fi
    echo "Testing what this branch touched:$(printf ' %s' $run)"
    echo "Dependents of these are not run here — main's run covers them."
    just rust-test-pkg $run

# Lint the packages this branch touched (pre-push's narrow clippy).
#
# `rust-clippy` is `--workspace --all-targets`, which compiles every
# target in every crate. It links nothing, so it is cheaper than the
# test suite, but it is still a whole-workspace build and it was still
# running in `pre-push` after the test half had been narrowed — the
# same defect, left half-fixed. Same scope rule, same limit: a lint
# that fires in a dependent crate is `main`'s run to report.
[group('check')]
[group('allow-agent')]
rust-clippy-changed:
    #!/usr/bin/env bash
    set -uo pipefail
    cd "{{ project_root }}"
    packages=$(just changed-packages) || exit 1
    if [ "$packages" = "--workspace" ]; then
        # Same split as `rust-test-changed`, for the same reason: in CI
        # there is no later run to defer to, so deferring is skipping.
        if [ -n "${CI:-}" ]; then
            echo "Every crate is in scope; linting the workspace."
            just rust-clippy
            exit "$?"
        fi
        echo "Every crate is in scope, so there is no narrow lint to run."
        echo "CI runs clippy over the workspace on this push; not here."
        exit 0
    fi
    if [ -z "$packages" ]; then
        echo "No workspace member changed; no Rust lint to run."
        exit 0
    fi
    echo "Linting what this branch touched:$(printf ' %s' $packages)"
    status=0
    for pkg in $packages; do
        echo
        echo "=== $pkg ==="
        # `-D warnings` and `--all-targets` are `rust-clippy`'s terms,
        # kept so that a lint passing here is one that passes there.
        cargo clippy -p "$pkg" --all-targets -- -D warnings || status=1
    done
    exit "$status"

# CI's recipe. Running it locally is discouraged, and no other recipe
# depends on it any more except `check`, which is CI's own entry point.
# It links every test binary in the workspace at once — one linker
# process each, gigabytes resident each, as many at a time as `jobs`
# allows. That is enough to put a machine into swap and take down
# whatever else is running on it, which on 2026-08-15 is exactly what
# it did to a shared box; but the shape of the cost is not specific to
# that machine, and a laptop pays it too. Reach for `rust-test-changed`
# or `rust-test-pkg` instead. This one is worth starting by hand when
# CI has reported something a narrow run cannot reproduce, when the
# change really is workspace-wide *and* the machine has the room, and
# not otherwise.
#
# Still the only sanctioned way to run the full suite when it is run at
# all. `cargo test --workspace`
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
#
# Run the whole workspace suite — CI's job; heavy on any machine.
[group('check')]
rust-test: rust-fmt-check
    #!/usr/bin/env bash
    set -uo pipefail
    # Said on the way in rather than in a comment nobody reads at the
    # moment they type it. Not a prompt and not a refusal: someone with
    # a reason to run this has one, and CI reaches it through `check`
    # with `CI` set.
    if [ -z "${CI:-}" ]; then
        echo "NOTE: this links every test binary in the workspace, which is" >&2
        echo "      minutes and gigabytes. 'just rust-test-changed' runs the" >&2
        echo "      packages this branch touched; CI runs this one on push." >&2
        echo >&2
    fi
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

# Wrap the repository's markdown at 80 columns.
#
# Three widths apply to this repository and only one of them is this: a
# commit message body wraps at 72, because `git log` indents it four
# spaces inside an 80-column terminal, and `commit-msg-check` is where
# that is enforced. A body written to be posted — a pull request, an
# issue — is not wrapped at all, because GitHub folds paragraphs itself
# and a hard-wrapped one arrives with breaks nobody put there on
# purpose; nothing here can check those, since they are never committed.
# What is left is the markdown in the tree, and 80 is the width the
# tools converged on (`markdownlint`'s MD013 defaults to it).
#
# Prettier rather than a checker of our own because the requirement is
# to fix, not to complain: `md-fmt` rewraps, `md-check` fails when
# something is unwrapped, and the two cannot disagree about what
# "wrapped" means. `.prettierrc.json` carries the width and
# `.prettierignore` carries what is exempt and why.
#
# It comes from the UI package because that is where this repository's
# node_modules already is; `cd crates/asterism-ui && npm ci` installs
# it, the same step `ui-test` needs.
[group('check')]
md-fmt:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{ project_root }}"
    exec "{{ ui_dir }}/node_modules/.bin/prettier" --write "**/*.md"

# Fail if any markdown in the tree is not wrapped at 80.
[group('check')]
md-check:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{ project_root }}"
    prettier="{{ ui_dir }}/node_modules/.bin/prettier"
    if [ ! -x "$prettier" ]; then
        echo "prettier is not installed. It comes with the UI package:" >&2
        echo "    cd crates/asterism-ui && npm ci" >&2
        exit 1
    fi
    "$prettier" --check "**/*.md" || {
        echo >&2
        echo "Run 'just md-fmt' to wrap them." >&2
        exit 1
    }

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

# Drive the team plane end to end, against a teams-server of its own.
#
# The suite `ui-e2e` cannot hold. Every read on the team plane is a
# request to a second binary, and this is the recipe that builds it;
# `wdio.teams.conf.ts` is what starts it, seeds a database nothing has
# touched, and stops it — including when the run fails.
#
# A run of its own rather than more specs under `ui-e2e`, because a run
# may hold one stateful fixture or two, and #188 is what two looks
# like: a spec that fails in a full run and passes alone, with the
# signature of a fixture left in one of two states. The separation is
# free here — the teams database is made empty per run and thrown away,
# which the app's own profile can never be.
#
# Same `allow-agent` terms as `ui-e2e`, and the same reason: an agent
# without it can only report that it did not verify, and this is the
# only surface that can check the team plane at all.
[group('check')]
[group('allow-agent')]
ui-e2e-teams: ffmpeg-sidecar
    #!/usr/bin/env bash
    set -euo pipefail
    # The second binary. Debug, because the config looks for it under
    # `target/debug/` beside the app the next command builds.
    cargo build -p teams-server
    cd "{{ ui_dir }}"
    # Same build shape as `ui-e2e` — see its comment for why `tauri
    # build` rather than `cargo build`.
    VITE_WDIO=1 npx tauri build --debug --no-bundle \
        --features wdio --config src-tauri/tauri.e2e.conf.json
    npx wdio run wdio.teams.conf.ts

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
