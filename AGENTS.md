# Melon Agent Guide

## Project Summary

Melon is a PTY-wrapping terminal autocomplete engine written in Rust. It spawns the user's shell inside a PTY, intercepts raw input, builds completion candidates from Fig-style specs plus runtime sources, and renders a popup directly in the terminal.

This is not a shell plugin and not a daemon. The core runtime is a proxy loop around a child shell.

## Primary Entry Points

- `src/main.rs`: CLI entrypoint, debug logging, `--install` shell snippet output.
- `src/lib.rs`: library surface for internal modules and benches.
- `src/pty/proxy.rs`: main proxy loop, popup state machine, PTY/stdin/stdout coordination.

## Repo Structure

- `src/completion/`
  - `spec.rs`: serde model for Fig-style specs and Melon completion candidates.
  - `loader.rs`: loads embedded and on-disk JSON specs.
  - `engine.rs`: parses completion context and builds candidate lists from specs.
  - `matcher.rs`: fuzzy ranking via `nucleo-matcher`.
  - `source.rs`: filesystem completion source.
  - `generator.rs`: runtime support for serializable Fig-like generators.
- `src/input/`
  - `parser.rs`: tokenization of the current command line.
  - `trigger.rs`: raw byte classification into input actions.
  - `line.rs`: cursor-aware editable line buffer and completion insertion logic.
- `src/pty/`
  - `proxy.rs`: interactive state machine.
  - `signals.rs`: SIGWINCH forwarding.
- `src/ui/`
  - `popup.rs`: selection and scroll state.
  - `render.rs`: ANSI popup rendering.
  - `theme.rs`: popup colors and layout.
- `src/shell/`
  - `detect.rs`: shell detection.
  - `cursor.rs`: cursor position helpers.
- `specs/`: embedded JSON command specs compiled into the binary with `include_dir`.
- `tools/convert_specs.ts`: converts upstream Fig/withfig TS specs into JSON.
- `benches/perf.rs`: local perf benchmarks.
- `tests/integration.rs`: integration coverage for spec loading.
- `docs/architecture.md`: architecture overview for contributors.
- `tasks/todo.md`: local implementation log and review notes.

## Runtime Model

Melon has two main interactive modes:

- `Passthrough`: input goes directly to the PTY while line state is tracked.
- `PopupActive`: navigation keys are intercepted, candidates are filtered, and Enter accepts a completion.

Completion flow:

1. `src/input/parser.rs` parses the current command segment.
2. `src/completion/engine.rs` resolves command/subcommand/arg context.
3. Static spec candidates, path candidates, and generator candidates are merged.
4. `src/completion/matcher.rs` ranks them.
5. `src/ui/render.rs` draws the popup.
6. `src/input/line.rs` applies the accepted completion edit back to the PTY.

## Spec Support

Melon supports a practical subset of Fig metadata:

- `displayName`
- `icon`
- `insertValue`
- `priority`
- static `suggestions`
- arg templates like `filepaths` and `folders`
- serializable generators with:
  - `script`
  - `splitOn`
  - `scriptTimeout`
  - string `trigger`
  - basic cache metadata

Current limits:

- Arbitrary JS is not supported.
- Function-valued Fig hooks such as `postProcess`, `custom`, `getQueryTerm`, and function-based generator logic are still out of scope.
- `tools/convert_specs.ts` preserves serializable generator metadata, but it is still a lightweight converter, not a full TS/AST importer.

## Build, Test, and Dev Commands

Run from the repo root:

- `cargo run`
- `cargo run -- --install`
- `cargo test -q`
- `cargo clippy -q`
- `cargo fmt --all`
- `cargo bench --bench perf`

Useful quick bench:

- `MELON_BENCH_ITERS=1000 cargo bench --bench perf`

## Editing Guidance

- Preserve the existing PTY proxy architecture. Avoid adding a second parallel input/completion pipeline.
- Keep completion candidate behavior consistent across static specs, path suggestions, and generator output.
- When changing completion acceptance, verify both inserted text and cursor placement.
- When changing popup behavior, inspect:
  - `src/pty/proxy.rs`
  - `src/input/line.rs`
  - `src/ui/popup.rs`
  - `src/ui/render.rs`
- When changing spec semantics, update:
  - `src/completion/spec.rs`
  - `src/completion/engine.rs`
  - `src/completion/matcher.rs`
  - `tools/convert_specs.ts`
- Specs are embedded at compile time. Changes under `specs/` require rebuild, not runtime reload.
- Leave existing user changes in `docs/architecture.md` intact unless explicitly asked to edit that file.

## Verification Expectations

For Rust code changes, run at least:

- `cargo fmt --all`
- `cargo test -q`
- `cargo clippy -q`

If the change touches completion latency or caches, also run:

- `cargo bench --bench perf`

If the change touches shell integration or popup interaction, do a manual check with:

- `cargo run -- --install`
- start a fresh shell
- exercise completion, popup navigation, accept/dismiss, and cwd-sensitive path completion

## Practical Notes

- Melon relies on OSC 7 from the shell integration snippet to track the shell cwd accurately.
- The binary sets `MELON=1` inside the wrapped shell to prevent recursive exec.
- The worktree may be dirty. Do not revert unrelated changes.
