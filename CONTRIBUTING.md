# Contributing to Melon

Thanks for your interest in contributing to Melon! This document covers the essentials to get you started.

For a deep dive into the codebase — architecture, module breakdown, and key design decisions — see [docs/architecture.md](docs/architecture.md).

## Prerequisites

- Rust (stable toolchain via `rustup`)
- macOS or Linux (Windows is not supported)
- `cargo` for building and testing

## Getting Started

```bash
git clone https://github.com/mrpbennett/melon.git
cd melon
cargo build
cargo test
```

Run melon locally:

```bash
cargo run               # Launch with your $SHELL inside a PTY
cargo run -- --debug    # Enable debug logging to ~/.local/share/melon/melon.log
```

## Project Layout

```
src/
├── pty/        # PTY proxy loop and state machine
├── input/      # Raw byte parsing and input classification
├── completion/ # Spec loading, engine, fuzzy matcher
├── ui/         # Popup rendering and theme
└── shell/      # Shell detection and cursor estimation

specs/          # JSON completion specs (35 included, embedded in binary)
tests/          # Integration tests
```

Full details in [docs/architecture.md](docs/architecture.md).

## Making Changes

1. Fork the repo and create a feature branch from `main`.
2. Make your changes, keeping diffs focused and minimal.
3. Run `cargo test` — all 35+ tests must pass.
4. Run `cargo clippy` and fix any warnings before submitting.
5. Open a pull request with a clear description of what changed and why.

## Adding a Completion Spec

1. Create `specs/<command>.json` following the [Fig autocomplete](https://github.com/withfig/autocomplete) format.
2. Run `cargo test` — the integration test validates all specs deserialize correctly.
3. Rebuild with `cargo build` — `include_dir` picks up the new file automatically.

See `specs/git.json` for a reference example.

## Coding Conventions

- Keep changes as small as possible — minimal blast radius.
- No `unwrap()` in production paths; propagate errors properly.
- Colocate unit tests in the relevant module under `#[cfg(test)] mod tests`.
- Prefer clarity over cleverness. This codebase is meant to be readable.

## Reporting Issues

Open a GitHub issue with:

- Your OS and shell (`echo $SHELL`, `uname -a`)
- Steps to reproduce
- Relevant output from `cargo run -- --debug` (log at `~/.local/share/melon/melon.log`)
