# Termpete Implementation

## Performance Pass
- [x] Establish a baseline for interactive hot paths and confirm where time is spent.
- [x] Refactor completion execution so popup typing does less repeated parsing and candidate rebuilding.
- [x] Cache filesystem completion results for stable directory listings and invalidate when the query context changes.
- [x] Reduce unnecessary flushes and temporary allocations in PTY/stdout forwarding and popup acceptance paths.
- [x] Extend tests to cover the refactor and verify with `cargo test`, `cargo clippy`, and targeted release timings.

### Performance Notes
- Current hotspots are PTY/stdout write flushing, completion recomputation on every popup keystroke, and synchronous `read_dir` calls for path completion.
- The spec corpus is small enough that a language rewrite is lower leverage than reducing per-keystroke I/O and allocation churn.
- Existing user edits in `docs/architecture.md` are out of scope and must remain untouched.

### Performance Review
- Completion parsing now happens once per request, and the engine caches base candidates by completion context so popup typing reuses the same candidate set when only the partial changes.
- Filesystem completion now caches sorted directory listings per base path and only re-reads when the base path changes.
- PTY/stdout forwarding now batches queued writes and avoids a per-message PTY flush.
- Verification: `rustfmt --edition 2021` on modified Rust files, `cargo test -q`, and `cargo clippy -q` all pass.

## Performance Pass 2
- [x] Reduce allocation churn in raw input classification and passthrough forwarding.
- [x] Batch contiguous passthrough bytes in the proxy so normal typing emits fewer channel sends and PTY writes.
- [x] Add a local benchmark target for completion latency and path-completion latency without adding network-only dependencies.
- [x] Verify with `cargo test -q`, `cargo clippy -q`, and `cargo bench --bench perf`.

### Performance Pass 2 Review
- `InputAction::Passthrough` is now allocation-free, and the proxy batches contiguous passthrough bytes before sending them across channels and into the PTY.
- Added `src/lib.rs` so internal modules can be benchmarked cleanly from a bench target without duplicating module wiring.
- Added `benches/perf.rs` and `[[bench]]` metadata so `cargo bench --bench perf` works on stable Rust with `harness = false`.
- Sample benchmark run with `MELON_BENCH_ITERS=1000 cargo bench --bench perf`:
- `completion.git_com`: 1.95 us average
- `completion.popup_typing`: 11.13 us average
- `path.cached_same_base`: 36.77 us average
- `path.alternating_base`: 261.08 us average

## Phase 1: PTY Proxy Skeleton ✅
- [x] `cargo init`, add dependencies (Cargo.toml)
- [x] `pty/proxy.rs` — spawn $SHELL in PTY, raw mode, async stdin↔PTY proxy
- [x] `pty/signals.rs` — SIGWINCH forwarding
- [x] `shell/detect.rs` — detect shell from $SHELL
- [x] `main.rs` — CLI with clap, launch proxy
- [x] Verify: `cargo build` succeeds

## Phase 2: Completion Data Model + Loader ✅
- [x] `completion/spec.rs` — Rust structs matching Fig spec format (serde)
- [x] `completion/loader.rs` — load JSON specs, index by command name + embedded via include_dir
- [x] `tools/convert_specs.ts` — Deno script to convert Fig TS specs → JSON
- [x] 35 hand-crafted specs (git, cargo, docker, npm, go, kubectl, gh, brew, etc.)
- [x] Verify: unit tests + integration test deserialize real specs

## Phase 3: Command Parser + Completion Engine ✅
- [x] `input/parser.rs` — tokenize command line (quotes, pipes, &&, escapes, redirects)
- [x] `completion/engine.rs` — walk spec tree, produce candidates for current context
- [x] `completion/source.rs` — CompletionSource trait, PathSource for filesystem
- [x] Verify: "git com" → [commit, compare, ...] (with matcher)

## Phase 4: Fuzzy Matching ✅
- [x] `completion/matcher.rs` — nucleo wrapper, filter + rank candidates
- [x] Verify: exact > prefix > fuzzy ordering

## Phase 5: Popup UI ✅
- [x] `ui/theme.rs` — Catppuccin-style colors, rounded borders, 8 items max
- [x] `ui/popup.rs` — selection state, scroll, wrap-around, page up/down
- [x] `ui/render.rs` — ANSI popup drawing (save/restore cursor, edge detection)
- [x] `shell/cursor.rs` — cursor position estimation

## Phase 6: Integration ✅
- [x] `input/trigger.rs` — Tab/Shift-Tab/arrows/Esc/Enter/Ctrl-C key classification
- [x] Wire proxy loop state machine (Passthrough → PopupActive → Accept/Dismiss)
- [x] Completion acceptance: backspace partial + insert completed text
- [x] `config.rs` — TOML config (max_visible, specs_dir)

## Phase 7: Polish ✅
- [x] Terminal edge handling (popup draws above if near bottom)
- [x] Ctrl-C/Ctrl-Z while popup open (dismiss + forward)
- [x] Filesystem completion (PathSource via template: filepaths/folders)
- [x] Logging via tracing (--debug flag writes to ~/.local/share/termpete/termpete.log)
- [x] `--install` flag (prints shell snippet for .zshrc/.bashrc)

## Review
- 0 warnings, 38 tests passing
- 36 command specs (git, cargo, docker, npm, go, kubectl, gh, brew, pip, tmux, jq, and more)
- Binary embeds all specs via include_dir — no external files needed at runtime
- Architecture: PTY wrapper with async channels + state machine for I/O interception
