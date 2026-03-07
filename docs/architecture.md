# Melon — Architecture

This document explains how Melon works internally. It is intended for contributors who want to understand the codebase before making changes.

## Overview

Melon is a PTY-wrapping terminal autocomplete engine. Rather than integrating with a specific shell, it sits between your terminal and your shell, intercepting raw input to show a floating completion popup.

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

## Key Design Decisions

**PTY wrapper, not a daemon or shell plugin** — Melon spawns your `$SHELL` inside a PTY and proxies I/O. This means it works with zsh, bash, fish, and anything else without per-shell integration scripts.

**Fig/Withfig spec format** — Completion data is stored as JSON following the [Fig autocomplete](https://github.com/withfig/autocomplete) schema. 35 specs are embedded in the binary at compile time via `include_dir`.

**`nucleo-matcher`** — SIMD-accelerated fuzzy matching from the Helix editor. Used for filtering and ranking candidates.

**`portable-pty`** — Cross-platform PTY handling from the WezTerm project.

**Async I/O via Tokio** — Stdin reader, PTY reader, and stdout writer run as separate tasks communicating over channels.

## State Machine (`src/pty/proxy.rs`)

The proxy runs a two-state machine:

```
┌─────────────┐   Tab (completions found)   ┌──────────────┐
│ Passthrough │ ─────────────────────────► │ PopupActive  │
│             │ ◄───────────────────────── │              │
└─────────────┘   Esc / Ctrl-C / typing     └──────────────┘
```

**Passthrough** — All input is forwarded to the PTY. Tracks `current_line` by watching printable chars, backspace, and enter. Tab triggers a completion lookup.

**PopupActive** — Tab/Down cycles the selection forward, Up/Shift-Tab goes backward, Enter accepts (backspaces the partial word and inserts the completion), Esc/Ctrl-C dismisses. Any other key dismisses the popup and passes the keystroke through.

## Completion Pipeline

```
current_line
    └─► parser::split_partial()       tokenize respecting quotes, pipes, &&
    └─► engine.complete()             walk the spec tree for the current context
    └─► matcher.filter(partial, candidates)   nucleo fuzzy scoring + sort
    └─► popup.set_items(scored)
    └─► renderer.render()             ANSI escape sequences to stdout
```

### Engine (`src/completion/engine.rs`)

1. Look up the command name in the `SpecStore`.
2. Walk subcommand tokens to find the deepest matching spec node.
3. Determine whether to complete subcommands, options, or positional args.
4. For args with `template: "filepaths"` or `"folders"`, delegate to `PathSource`.

## Module Reference

| Path                        | Responsibility                                        |
| --------------------------- | ----------------------------------------------------- |
| `src/main.rs`               | CLI (clap), logging setup, entry point                |
| `src/config.rs`             | `~/.config/melon/config.toml` loading                 |
| `src/pty/proxy.rs`          | Core proxy loop + state machine                       |
| `src/pty/signals.rs`        | SIGWINCH forwarding to PTY                            |
| `src/input/parser.rs`       | Command-line tokenizer (quotes, pipes, `&&`, escapes) |
| `src/input/trigger.rs`      | Raw byte → `InputAction` classification               |
| `src/completion/spec.rs`    | Rust structs for Fig spec types (serde)               |
| `src/completion/loader.rs`  | Load specs from disk + embedded                       |
| `src/completion/engine.rs`  | Spec tree walker → candidates                         |
| `src/completion/matcher.rs` | nucleo fuzzy matching wrapper                         |
| `src/completion/source.rs`  | `CompletionSource` trait, `PathSource`                |
| `src/ui/popup.rs`           | Selection state, scroll, wrap-around navigation       |
| `src/ui/render.rs`          | ANSI escape sequence popup drawing                    |
| `src/ui/theme.rs`           | Colors (Catppuccin dark), rounded borders, dimensions |
| `src/shell/detect.rs`       | `$SHELL` detection (zsh/bash/fish)                    |
| `src/shell/cursor.rs`       | Cursor position estimation for popup placement        |

## Spec Format

Specs follow the [Fig autocomplete](https://github.com/withfig/autocomplete) format (camelCase JSON):

```json
{
  "name": ["git", "g"],
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
  "options": [],
  "args": {
    "template": "filepaths",
    "isVariadic": true
  }
}
```

Key types: `Spec`, `Subcommand`, `Opt`, `Arg`, `StringOrArray`, `ArgOrArgs`, `Template`, `SuggestionOrString` — all in `src/completion/spec.rs`.

## Environment Variables

| Variable  | Purpose                                                                      |
| --------- | ---------------------------------------------------------------------------- |
| `MELON=1` | Set inside the PTY so the shell knows it's wrapped (prevents recursive exec) |
| `SHELL`   | Used to detect which shell to spawn                                          |
| `TERM`    | Passed through to the child shell                                            |

## Testing

- Unit tests are colocated in each module under `#[cfg(test)] mod tests`.
- `tests/integration.rs` validates all 35 spec files deserialize without error.
- Run everything with `cargo test`.
