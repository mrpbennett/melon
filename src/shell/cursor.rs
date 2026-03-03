/// Get the current cursor position from the host terminal.
/// Uses crossterm's built-in DSR query.
///
/// Note: This only works reliably when called from the main thread
/// with raw mode enabled and no concurrent stdin reads.
/// In the proxy, we estimate position instead.
pub fn estimate_cursor_row() -> u16 {
    // During the proxy loop, we can't reliably do DSR because stdin
    // is being read by another thread. Instead we estimate using
    // terminal size — the prompt is typically near the bottom.
    let (_, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    // The shell prompt is usually at the current cursor line.
    // We'll use a conservative estimate: assume cursor is at the
    // bottom portion of the terminal.
    rows.saturating_sub(2)
}
