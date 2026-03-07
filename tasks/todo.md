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

## UX Pass
- [x] Track the active shell working directory via OSC 7 and use it for relative path completion.
- [x] Replace the append-only line model with a cursor-aware editable line buffer that handles mid-line editing.
- [x] Preserve popup selection across re-filtering when the selected candidate still exists.
- [x] Accept completions by replacing the token around the cursor and appending a trailing space only when appropriate.
- [x] Verify with `cargo test -q`, `cargo clippy -q`, and targeted benchmark/manual sanity checks.

### UX Pass Review
- Added a cursor-aware `LineState` so melon can keep editing state around the cursor instead of assuming append-only input.
- The proxy now refreshes popup completions against `line_state.before_cursor()` and preserves selection while typing, backspacing, deleting, and moving the cursor.
- Added OSC 7 shell integration in the install snippet and PTY-side OSC 7 parsing so relative path completion follows the wrapped shell's actual working directory.
- Completion acceptance now replaces the token around the cursor, avoids clobbering text outside that token, and only appends a trailing space when the inserted completion ends at the buffer boundary and is not a folder.
- Verification passed with `cargo fmt --all`, `cargo test -q`, `cargo clippy -q`, `MELON_BENCH_ITERS=1000 cargo bench --bench perf`, and an install-snippet sanity check via `cargo run -q -- --install`.

## Emoji/Icon Pass
- [x] Confirm how Fig encodes emoji and icon metadata in specs and map the relevant subset onto Melon's JSON/spec model.
- [x] Extend Melon's spec structs and completion candidates to preserve `icon` and `displayName` metadata for subcommands, options, and suggestions.
- [x] Render terminal-safe icons in the popup, including direct emoji strings and sensible fallbacks for `fig://icon?...` and URL values.
- [x] Verify with focused unit tests plus `cargo test -q`, `cargo clippy -q`, and a manual sanity check of rendered output behavior.

### Emoji/Icon Pass Review
- Fig treats `Suggestion` as the shared UI base type, and `Subcommand`/`Option` inherit `icon` and `displayName`. The docs also show `icon` accepts a single-character string, an emoji, a URL, or `fig://icon?...`.
- The `withfig/autocomplete` `git` spec uses both direct emoji icons (`⭐️`, `🏷️`) and `fig://icon?...` values in generated suggestions, so Melon now preserves both forms at the candidate level.
- Melon now deserializes `displayName` and `icon` on command specs, subcommands, options, and structured suggestions, then carries them through the completion engine into popup rendering.
- The terminal renderer uses raw emoji icons verbatim, maps common `fig://icon?...` types to terminal-safe glyphs, falls back to generic icons for unknown protocols, and displays `displayName` without changing the inserted `name`.
- Limitation: Melon's static JSON conversion still strips generator functions, so dynamic Fig generator output is not imported from upstream specs. The renderer and candidate model now support those icon values if a future runtime data source emits them.
- Verification passed with `cargo fmt --all`, `cargo test -q`, and `cargo clippy -q`.

## Fig Metadata Pass
- [x] Preserve Fig `insertValue` and `priority` metadata in the Rust spec model and completion candidates.
- [x] Use `insertValue` during completion acceptance and `priority` during ranking/tie-breaking.
- [x] Add a serializable generator runtime for JSON specs, including command execution, caching, and terminal-safe output mapping.
- [x] Teach `tools/convert_specs.ts` to preserve the supported generator subset instead of dropping it outright.
- [x] Verify with focused tests plus `cargo fmt --all`, `cargo test -q`, and `cargo clippy -q`.

### Fig Metadata Notes
- Scope the generator work to a serializable subset that can round-trip through JSON: string `script`, `splitOn`, `scriptTimeout`, string `trigger`, simple cache metadata, and fixed output field mapping.
- Function-based Fig features such as `script(tokens)`, `postProcess`, `custom`, function-valued `trigger`, and `getQueryTerm` remain out of scope until Melon has a JS runtime or dedicated reimplementation.

### Fig Metadata Review
- Melon now preserves `insertValue` and `priority` on commands, subcommands, options, and structured suggestions, then uses them for popup ordering and insertion.
- Completion acceptance now parses Fig-style `insertValue` strings, including `{cursor}` cursor placement and backspace/newline control characters, before emitting PTY edits.
- Added a generator source that executes serializable command generators with `splitOn`, `scriptTimeout`, string `trigger`, cache metadata, and path-template fallback. Generator stdout can also be JSON arrays of Fig-style suggestions, which lets dynamic candidates carry icons, descriptions, `insertValue`, and `priority`.
- `tools/convert_specs.ts` now preserves serializable `generators` and string `trigger` data instead of discarding those keys up front; unsupported function-valued hooks are still stripped.
- Verification passed with `cargo fmt --all`, `cargo test -q`, and `cargo clippy -q`.

## Option And Shell UX Pass
- [x] Inherit persistent options from parent specs and filter option candidates based on already-used, repeatable, and mutually exclusive flags.
- [x] Use the same option scope for option-arg completion so inherited persistent flags resolve correctly.
- [x] Replace the raw token span around the cursor during accept instead of deleting the already-unescaped logical partial.
- [x] Escape inserted candidate text according to the active shell quoting context when the candidate does not provide an explicit `insertValue`.
- [x] Verify with focused unit tests plus `cargo fmt --all`, `cargo test -q`, and `cargo clippy -q`.

### Option And Shell UX Notes
- Prefer a single option-scope implementation inside the completion engine so inheritance and filtering rules stay consistent across option listing and option-arg lookup.
- Preserve explicit spec-provided `insertValue` strings exactly; only shell-escape fallback insertions derived from the candidate `name`.
- Preserve surrounding open quotes during replacement when the user is completing inside a quoted token.

### Option And Shell UX Review
- The completion engine now builds an inherited option scope that keeps current-node options, adds persistent parent/root options, and filters hidden, already-used non-repeatable, and mutually exclusive flags before they reach the popup.
- Option-arg lookup now uses that same scope, so persistent root options like `--config` continue to resolve suggestion/path/generator args under subcommands.
- Popup acceptance now computes a raw replacement span around the cursor instead of deleting the already-unescaped logical partial, which fixes acceptance for escaped tokens like `hello\ world` and preserves surrounding open quotes.
- Fallback insertions derived from `candidate.name` are now escaped for unquoted, single-quoted, and double-quoted shell contexts. Explicit spec-provided `insertValue` strings are still inserted verbatim.
- Verification passed with `cargo fmt --all`, `cargo test -q`, and `cargo clippy -q`.
- Manual interactive shell validation was not run in this pass.

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
