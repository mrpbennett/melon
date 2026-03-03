# Termpete Implementation

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
- 0 warnings, 35 tests passing
- 35 command specs (git, cargo, docker, npm, go, kubectl, gh, brew, pip, tmux, jq, and more)
- Binary embeds all specs via include_dir — no external files needed at runtime
- Architecture: PTY wrapper with async channels + state machine for I/O interception
