# Melon

Warp-style terminal autocomplete engine that works in **any** terminal. Written in Rust. Wraps your shell in a PTY and intercepts Tab to show a floating completion popup with commands, descriptions, and fuzzy matching.

## Quick Reference

```bash
cargo build              # Build
cargo test               # Run all tests (35 tests)
cargo run                # Launch melon (spawns your $SHELL inside a PTY)
cargo run -- --debug     # Launch with debug logging to ~/.local/share/melon/melon.log
cargo run -- --install   # Print shell integration snippet
```

## Architecture

```
Host Terminal (raw mode)
    │                           │
    │ stdin                     │ stdout
    ▼                           ▲
┌──────────────────────────────────────┐
│            melon process             │
│                                      │
│  stdin ─► InputProcessor ─┬─► PTY    │
│           (state machine)  │  master  │
│                            │         │
│                    CompletionEngine  │
│                            │         │
│                      PopupRenderer ──► host stdout
│                            │         │
│  PTY master reader ────────┴─► host stdout
└──────────────────────────────────────┘
```

**Key design decisions:**
- **PTY wrapper** (not daemon+plugin) — works with any shell/terminal, no per-shell integration needed
- **Fig/Withfig completion specs** as data format — JSON specs, 35 commands included
- **`nucleo-matcher`** for fuzzy matching (from Helix editor, SIMD-accelerated)
- **`portable-pty`** (from wezterm) for cross-platform PTY handling
- **`crossterm`** for raw terminal control and ANSI rendering
- **Async I/O** via tokio with channels between stdin reader, PTY, and stdout writer
- **Specs embedded in binary** via `include_dir` — no external files needed at runtime

## Project Structure

```
src/
├── main.rs                    # CLI (clap), logging setup, entry point
├── config.rs                  # ~/.config/melon/config.toml loading
├── pty/
│   ├── mod.rs
│   ├── proxy.rs               # Core proxy loop + state machine (Passthrough ↔ PopupActive)
│   └── signals.rs             # SIGWINCH forwarding to PTY
├── input/
│   ├── mod.rs
│   ├── parser.rs              # Command-line tokenizer (quotes, pipes, &&, escapes)
│   └── trigger.rs             # Raw byte → InputAction classification (Tab, arrows, Esc, etc.)
├── completion/
│   ├── mod.rs
│   ├── spec.rs                # Rust structs mirroring Fig spec types (serde)
│   ├── loader.rs              # Load JSON specs from disk + embedded (include_dir)
│   ├── engine.rs              # Spec tree walker → candidates for current input context
│   ├── matcher.rs             # nucleo fuzzy matching wrapper, filter + rank
│   └── source.rs              # CompletionSource trait, PathSource for filesystem
├── ui/
│   ├── mod.rs
│   ├── popup.rs               # Selection state, scroll, wrap-around navigation
│   ├── render.rs              # ANSI escape sequence popup drawing
│   └── theme.rs               # Colors (Catppuccin dark), rounded borders, dimensions
└── shell/
    ├── mod.rs
    ├── detect.rs              # $SHELL detection (zsh/bash/fish)
    └── cursor.rs              # Cursor position estimation for popup placement

specs/                         # 35 JSON completion specs (embedded in binary at compile time)
tools/convert_specs.ts         # Deno script: convert Fig TypeScript specs → JSON
tests/integration.rs           # Integration test: validates all spec files deserialize
tasks/todo.md                  # Implementation tracking
```

## State Machine

The proxy (`src/pty/proxy.rs`) runs a two-state machine:

1. **Passthrough** — all input goes directly to the PTY. Tracks `current_line` by watching for printable chars, backspace, and enter. Tab triggers completion lookup.
2. **PopupActive** — Tab/Down cycles selection, Up/Shift-Tab goes backwards, Enter accepts (backspaces the partial then inserts the completion), Esc/Ctrl-C dismisses. Any typing dismisses and passes through.

## Completion Pipeline

```
current_line → parser::split_partial()
            → engine.complete() (walks spec tree for subcommands/options/args)
            → matcher.filter(partial, candidates) (nucleo fuzzy scoring)
            → popup.set_items(scored) → renderer.render()
```

The engine resolves context by:
1. Looking up the command name in the `SpecStore`
2. Walking subcommand tokens to find the deepest matching spec node
3. Determining whether to complete subcommands, options, or positional args
4. For args with `template: "filepaths"/"folders"`, delegating to `PathSource`

## Spec Format

Specs follow the [Fig autocomplete](https://github.com/withfig/autocomplete) format (camelCase JSON):

```json
{
  "name": ["git", "g"],        // string or array of aliases
  "description": "Version control",
  "subcommands": [
    {
      "name": "commit",
      "description": "Record changes",
      "options": [
        {
          "name": ["-m", "--message"],
          "description": "Commit message",
          "args": { "name": "message" }
        }
      ]
    }
  ],
  "options": [...],
  "args": {
    "template": "filepaths",   // "filepaths" | "folders" for filesystem completion
    "isVariadic": true,
    "suggestions": ["value1", {"name": "value2", "description": "..."}]
  }
}
```

Key spec types: `Spec`, `Subcommand`, `Opt`, `Arg`, `StringOrArray`, `ArgOrArgs`, `Template`, `SuggestionOrString` — all defined in `src/completion/spec.rs`.

## Adding New Command Specs

1. Create `specs/<command>.json` following the format above
2. Rebuild — `include_dir` picks it up automatically
3. Or use the converter: `deno run --allow-read --allow-write tools/convert_specs.ts /path/to/fig-autocomplete/src specs/`

Users can also drop custom specs into `~/.local/share/melon/specs/` (loaded at runtime, overrides builtins).

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `MELON=1` | Set inside the PTY so the shell knows it's wrapped (prevents recursive exec) |
| `SHELL` | Used to detect which shell to spawn |
| `TERM` | Passed through to the child shell |

## Config

Optional `~/.config/melon/config.toml`:

```toml
max_visible = 8          # Max popup items (default: 8)
specs_dir = "/path/to/specs"  # Custom specs directory
```

## Testing

- Unit tests are colocated in each module (`#[cfg(test)] mod tests`)
- Integration test in `tests/integration.rs` validates all 35 spec files parse correctly
- Key test coverage: tokenizer (quotes, pipes, escapes), spec deserialization, engine (subcommand/option completion), fuzzy matcher (scoring order), popup state (selection wrapping, scroll), input classifier (key mapping)

## Dependencies

| Crate | Purpose |
|-------|---------|
| `portable-pty` | PTY creation and management |
| `crossterm` | Terminal raw mode, size query, ANSI rendering |
| `nucleo-matcher` | SIMD-accelerated fuzzy matching |
| `tokio` | Async runtime, channels, task spawning |
| `serde` / `serde_json` | Spec deserialization |
| `clap` | CLI argument parsing |
| `include_dir` | Embed specs/ directory in binary |
| `signal-hook` / `signal-hook-tokio` | SIGWINCH handling |
| `tracing` / `tracing-subscriber` | Debug logging |
| `dirs` | XDG directory resolution |
| `toml` | Config file parsing |
| `unicode-width` | Correct column width for popup layout |

## Platform

macOS and Linux. Uses Unix PTY APIs via `portable-pty` and `nix`. Not Windows-compatible (no Windows PTY support in current architecture).
