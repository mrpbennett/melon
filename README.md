<h1 align="center">
    <img src="assets/imgs/melon-logo.png" width="200" alt="Logo"/><br/>
    melon
</h1>

![demo-video](assets/demo-video.mp4)

---

Melon wraps your shell in a PTY and intercepts Tab to show a floating completion popup with commands, descriptions, and fuzzy matching — no per-shell plugin required.

## Installation

**Prerequisites:** Rust 1.70+ ([install rustup](https://rustup.rs))

```bash
cargo install --git https://github.com/mrpbennett/melon
```

This builds melon and places it in `~/.cargo/bin/melon`. Make sure `~/.cargo/bin` is in your `$PATH`.

To update to the latest version, run the same command again.

## Usage

Launch melon from any terminal:

```bash
melon --install
```

This will populate the following:

```
# Add this to your shell rc file (~/.zshrc or ~/.bashrc):
# It wraps your shell in melon for autocomplete support.

if [ -z "$MELON" ] && [ -t 0 ] && [ -t 1 ]; then
  exec /Users/paul/.cargo/bin/melon
fi
```

Everything should work as normal — the only difference is that pressing **Tab** opens a fuzzy completion popup instead of running the shell's built-in completion.

### Popup controls

| Key               | Action           |
| ----------------- | ---------------- |
| `Tab` / `↓`       | Next item        |
| `Ctrl+j`          | Next item        |
| `Shift+Tab` / `↑` | Previous item    |
| `Ctrl+k`          | Previous item    |
| `Enter`           | Accept selection |
| `Esc` / `Ctrl+C`  | Dismiss popup    |

## Custom specs

Melon ships with 36 built-in completion specs (git, docker, cargo, npm, kubectl, claude, and more). You can add your own or override builtins by dropping JSON files into:

```
~/.local/share/melon/specs/
```

No rebuild required — specs are loaded at startup.

Specs follow the [Fig autocomplete](https://github.com/withfig/autocomplete) format:

```json
{
  "name": "mytool",
  "description": "My custom tool",
  "subcommands": [{ "name": "run", "description": "Run the thing" }],
  "options": [{ "name": ["--verbose", "-v"], "description": "Verbose output" }]
}
```

## Configuration

Optional config file at `~/.config/melon/config.toml`:

```toml
max_visible = 8          # Max items shown in the popup (default: 8)
specs_dir = "~/.local/share/melon/specs"  # Custom specs directory
```

## Building from source

```bash
git clone https://github.com/mrpbennett/melon
cd melon
cargo build --release
./target/release/melon
```

## Debugging

```bash
melon --debug
# Logs written to ~/.local/share/melon/melon.log
```

## Platform

macOS and Linux. Requires a Unix PTY — not compatible with Windows.

## Inspiration

- [https://github.com/StanMarek/ghost-complete](https://github.com/StanMarek/ghost-complete)
