use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};

use crate::completion::engine::CompletionEngine;
use crate::completion::loader::SpecStore;
use crate::completion::matcher::FuzzyMatcher;
use crate::config::Config;
use crate::input::trigger::{classify_input, InputAction};
use crate::shell::detect::detect_shell;
use crate::ui::popup::PopupState;
use crate::ui::render::PopupRenderer;
use crate::ui::theme::Theme;

/// Proxy mode state machine.
#[derive(Debug, PartialEq)]
enum Mode {
    /// Normal passthrough — all input goes to PTY.
    Passthrough,
    /// Popup is active — intercept navigation keys.
    PopupActive,
}

/// Remove the last word from `line`, matching shell kill-word (Ctrl+W) behaviour:
/// trim trailing spaces, then delete back to the previous space boundary.
fn kill_last_word(line: &mut String) {
    // Trim trailing whitespace
    while line.ends_with(' ') {
        line.pop();
    }
    // Delete back to (but not including) the previous space
    while !line.is_empty() && !line.ends_with(' ') {
        line.pop();
    }
}

fn drain_channel_batch(receiver: &mut mpsc::Receiver<Vec<u8>>, mut batch: Vec<u8>) -> Vec<u8> {
    while let Ok(next) = receiver.try_recv() {
        batch.extend_from_slice(&next);
    }
    batch
}

fn track_passthrough_in_passthrough_mode(current_line: &mut String, bytes: &[u8]) {
    if bytes.first() == Some(&0x1b) {
        return;
    }

    for &b in bytes {
        if b == b'\r' || b == b'\n' {
            current_line.clear();
        } else if b == 0x7f || b == 0x08 {
            current_line.pop();
        } else if b >= 0x20 {
            current_line.push(b as char);
        }
    }
}

fn track_passthrough_in_popup(current_line: &mut String, bytes: &[u8]) {
    for &b in bytes {
        if b >= 0x20 {
            current_line.push(b as char);
        }
    }
}

async fn flush_passthrough_buffer(pty_tx: &mpsc::Sender<Vec<u8>>, passthrough_buf: &mut Vec<u8>) {
    if passthrough_buf.is_empty() {
        return;
    }

    let pending = std::mem::take(passthrough_buf);
    let _ = pty_tx.send(pending).await;
}

struct PopupRefreshState<'a> {
    renderer: &'a PopupRenderer,
    popup: &'a mut PopupState,
    stdout_tx: &'a mpsc::Sender<Vec<u8>>,
    popup_row: u16,
    popup_col: u16,
    popup_col_actual: &'a mut u16,
    popup_lines: &'a mut u16,
    popup_partial_len: &'a mut usize,
    mode: &'a mut Mode,
}

async fn refresh_popup_completion(
    engine: &mut CompletionEngine,
    matcher: &mut FuzzyMatcher,
    current_line: &str,
    state: PopupRefreshState<'_>,
) {
    let PopupRefreshState {
        renderer,
        popup,
        stdout_tx,
        popup_row,
        popup_col,
        popup_col_actual,
        popup_lines,
        popup_partial_len,
        mode,
    } = state;

    let completion = engine.complete(current_line);
    let scored = matcher.filter(&completion.partial, completion.candidates);

    if !scored.is_empty() {
        *popup_partial_len = completion.partial.len();
        let mut render_buf = Vec::new();
        let _ = renderer.clear(&mut render_buf, popup_row, *popup_col_actual, *popup_lines);
        popup.set_items(scored);
        let (lines, col) = renderer
            .render(&mut render_buf, popup, popup_row, popup_col)
            .unwrap_or((0, *popup_col_actual));
        *popup_lines = lines;
        *popup_col_actual = col;
        let _ = stdout_tx.send(render_buf).await;
    } else {
        let mut clear_buf = Vec::new();
        let _ = renderer.clear(&mut clear_buf, popup_row, *popup_col_actual, *popup_lines);
        let _ = stdout_tx.send(clear_buf).await;
        popup.dismiss();
        *popup_lines = 0;
        *popup_partial_len = 0;
        *mode = Mode::Passthrough;
    }
}

/// Spawn the user's shell inside a PTY and proxy I/O between the host terminal
/// and the child PTY. Returns the child exit code.
pub async fn run_proxy() -> Result<i32> {
    let (shell_path, shell_type) = detect_shell();
    tracing::info!(?shell_path, ?shell_type, "detected shell");

    let config = Config::load().unwrap_or_default();

    // Load completion specs
    let mut store = SpecStore::new();
    // Load built-in specs (embedded in binary)
    let builtin_count = store.load_builtin();
    tracing::info!(builtin_count, "loaded builtin specs");
    // Load user specs (override/extend builtins)
    let specs_dir = config.specs_dir();
    if specs_dir.exists() {
        let _ = store.load_dir(&specs_dir);
    }
    let cmd_count = store.len();
    tracing::info!(cmd_count, "total completion specs loaded");

    let mut engine = CompletionEngine::new(store);
    let mut matcher = FuzzyMatcher::new();

    // Get current terminal size
    let (cols, rows) = crossterm::terminal::size().context("failed to get terminal size")?;

    // Create PTY
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("failed to open PTY")?;

    // Build shell command
    let mut cmd = CommandBuilder::new(&shell_path);
    cmd.env(
        "TERM",
        std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string()),
    );
    cmd.env("MELON", "1");

    // Spawn the shell
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .context("failed to spawn shell")?;

    drop(pair.slave);

    let master = pair.master;

    let mut pty_reader = master
        .try_clone_reader()
        .context("failed to clone PTY reader")?;
    let mut pty_writer = master.take_writer().context("failed to take PTY writer")?;

    // Put host terminal into raw mode
    crossterm::terminal::enable_raw_mode().context("failed to enable raw mode")?;

    // Query actual cursor position via DSR (Device Status Report) before stdin thread starts.
    // crossterm writes \x1b[6n and reads the \x1b[{row};{col}R response directly from stdin,
    // which is safe here because the stdin reader thread hasn't started yet.
    let (init_col, init_row) = crossterm::cursor::position().unwrap_or((0, 0));

    let shutdown = Arc::new(Notify::new());

    // Channel: raw stdin bytes → main loop
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(64);
    // Channel: main loop → PTY writer
    let (pty_tx, mut pty_rx) = mpsc::channel::<Vec<u8>>(64);

    // Task: stdin reader → channel
    let shutdown_stdin = shutdown.clone();
    let stdin_handle = tokio::task::spawn_blocking(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if stdin_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::Interrupted {
                        break;
                    }
                }
            }
        }
        shutdown_stdin.notify_one();
    });

    // Task: channel → PTY writer
    let pty_write_handle = tokio::task::spawn_blocking(move || {
        while let Some(data) = pty_rx.blocking_recv() {
            let batch = drain_channel_batch(&mut pty_rx, data);
            if pty_writer.write_all(&batch).is_err() {
                break;
            }
        }
    });

    // Channel: popup renders + forwarded PTY output → stdout writer
    let (stdout_tx, mut stdout_rx) = mpsc::channel::<Vec<u8>>(64);

    // Task: PTY reader → pty_out channel (main loop tracks cursor + forwards to stdout)
    let (pty_out_tx, mut pty_out_rx) = mpsc::channel::<Vec<u8>>(64);

    let shutdown_stdout = shutdown.clone();
    let pty_read_handle = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 4096];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if pty_out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::Interrupted {
                        break;
                    }
                }
            }
        }
        shutdown_stdout.notify_one();
    });

    // Task: stdout channel → actual stdout
    let stdout_write_handle = tokio::task::spawn_blocking(move || {
        let mut stdout = std::io::stdout().lock();
        while let Some(data) = stdout_rx.blocking_recv() {
            let batch = drain_channel_batch(&mut stdout_rx, data);
            if stdout.write_all(&batch).is_err() {
                break;
            }
            if stdout.flush().is_err() {
                break;
            }
        }
    });

    // Handle SIGWINCH
    let master_resize = master;
    let sigwinch_handle = tokio::spawn(async move {
        crate::pty::signals::forward_sigwinch(master_resize).await;
    });

    // Wait for child exit in background
    let (child_tx, mut child_rx) = mpsc::channel::<bool>(1);
    let child_wait_handle = tokio::task::spawn_blocking(move || {
        let status = child.wait();
        let success = status.map(|s| s.success()).unwrap_or(false);
        let _ = child_tx.blocking_send(success);
    });

    // === Main state machine loop ===
    let mut theme = Theme::default();
    if let Some(v) = config.show_description_panel {
        theme.show_description_panel = v;
    }
    let max_visible = config.max_visible.unwrap_or(theme.max_visible);
    let renderer = PopupRenderer::new(theme);
    let mut popup = PopupState::new(max_visible);
    let mut mode = Mode::Passthrough;
    let mut current_line = String::new();
    let mut popup_lines: u16 = 0;
    let mut popup_partial_len: usize = 0;
    // Cursor tracking: initialised from DSR query, updated via PTY output newlines.
    let mut last_cursor_row: u16 = init_row;
    let mut last_cursor_col: u16 = init_col;
    // Snapshot of cursor position when the popup was opened (used for render/clear).
    let mut popup_row: u16 = 0;
    let mut popup_col: u16 = 0;
    // Actual column render() placed the popup at (may differ from popup_col when near right edge).
    let mut popup_col_actual: u16 = 0;
    let mut child_exited = false;

    loop {
        tokio::select! {
            // Input from stdin
            Some(raw) = stdin_rx.recv() => {
                let mut offset = 0;
                let mut passthrough_buf = Vec::new();
                while offset < raw.len() {
                    let (action, consumed) = classify_input(&raw[offset..]);
                    if consumed == 0 { break; }
                    offset += consumed;
                    let bytes = &raw[offset - consumed..offset];

                    if !matches!(action, InputAction::Passthrough) && !passthrough_buf.is_empty() {
                        flush_passthrough_buffer(&pty_tx, &mut passthrough_buf).await;
                        if mode == Mode::PopupActive {
                            refresh_popup_completion(&mut engine, &mut matcher, &current_line, PopupRefreshState {
                                renderer: &renderer,
                                popup: &mut popup,
                                stdout_tx: &stdout_tx,
                                popup_row,
                                popup_col,
                                popup_col_actual: &mut popup_col_actual,
                                popup_lines: &mut popup_lines,
                                popup_partial_len: &mut popup_partial_len,
                                mode: &mut mode,
                            })
                            .await;
                        }
                    }

                    match mode {
                        Mode::Passthrough => {
                            match action {
                                InputAction::Tab => {
                                    // Trigger completion
                                    if !current_line.trim().is_empty() {
                                        let completion = engine.complete(&current_line);
                                        let scored =
                                            matcher.filter(&completion.partial, completion.candidates);
                                        if !scored.is_empty() {
                                            popup_partial_len = completion.partial.len();
                                            popup.set_items(scored);
                                            mode = Mode::PopupActive;
                                            // Snapshot cursor position for this popup's lifetime.
                                            // last_cursor_row is maintained from DSR + PTY newline tracking.
                                            popup_row = last_cursor_row;
                                            popup_col = last_cursor_col;
                                            // Render popup; capture the actual column it was drawn at.
                                            let mut render_buf = Vec::new();
                                            let (lines, col) = renderer.render(&mut render_buf, &popup, popup_row, popup_col).unwrap_or((0, popup_col));
                                            popup_lines = lines;
                                            popup_col_actual = col;
                                            let _ = stdout_tx.send(render_buf).await;
                                        } else {
                                            popup_partial_len = 0;
                                            // No completions — pass tab through
                                            let _ = pty_tx.send(vec![0x09]).await;
                                        }
                                    } else {
                                        let _ = pty_tx.send(vec![0x09]).await;
                                    }
                                }
                                InputAction::Passthrough => {
                                    track_passthrough_in_passthrough_mode(&mut current_line, bytes);
                                    passthrough_buf.extend_from_slice(bytes);
                                }
                                InputAction::Enter => {
                                    current_line.clear();
                                    let _ = pty_tx.send(vec![0x0d]).await;
                                }
                                InputAction::Backspace => {
                                    current_line.pop();
                                    let _ = pty_tx.send(vec![0x7f]).await;
                                }
                                InputAction::CtrlC => {
                                    current_line.clear();
                                    let _ = pty_tx.send(vec![0x03]).await;
                                }
                                InputAction::CtrlZ => {
                                    current_line.clear();
                                    let _ = pty_tx.send(vec![0x1a]).await;
                                }
                                InputAction::CtrlJ => {
                                    // No popup — pass LF through to PTY
                                    let _ = pty_tx.send(vec![0x0a]).await;
                                }
                                InputAction::CtrlK => {
                                    // No popup — pass through to PTY
                                    let _ = pty_tx.send(vec![0x0b]).await;
                                }
                                InputAction::KillWord => {
                                    kill_last_word(&mut current_line);
                                    let _ = pty_tx.send(raw[offset - consumed..offset].to_vec()).await;
                                }
                                InputAction::KillLine => {
                                    current_line.clear();
                                    let _ = pty_tx.send(raw[offset - consumed..offset].to_vec()).await;
                                }
                                _ => {
                                    // Pass through everything else
                                    let _ = pty_tx.send(raw[offset - consumed..offset].to_vec()).await;
                                }
                            }
                        }
                        Mode::PopupActive => {
                            match action {
                                InputAction::Down | InputAction::Tab | InputAction::CtrlJ => {
                                    popup.select_next();
                                    let mut render_buf = Vec::new();
                                    let (lines, col) = renderer.render(&mut render_buf, &popup, popup_row, popup_col).unwrap_or((0, popup_col_actual));
                                    popup_lines = lines;
                                    popup_col_actual = col;
                                    let _ = stdout_tx.send(render_buf).await;
                                }
                                InputAction::Up | InputAction::ShiftTab | InputAction::CtrlK => {
                                    popup.select_prev();
                                    let mut render_buf = Vec::new();
                                    let (lines, col) = renderer.render(&mut render_buf, &popup, popup_row, popup_col).unwrap_or((0, popup_col_actual));
                                    popup_lines = lines;
                                    popup_col_actual = col;
                                    let _ = stdout_tx.send(render_buf).await;
                                }
                                InputAction::Enter => {
                                    // Accept the selected completion
                                    if let Some(text) = popup.selected_text() {
                                        let text = text.to_string();
                                        // Clear popup first
                                        let mut clear_buf = Vec::new();
                                        let _ = renderer.clear(&mut clear_buf, popup_row, popup_col_actual, popup_lines);
                                        let _ = stdout_tx.send(clear_buf).await;

                                        // Calculate how much to backspace (remove partial)
                                        let backspaces = popup_partial_len;

                                        // Send backspaces to delete partial
                                        let mut edit_bytes = vec![0x7f; backspaces];
                                        // Then send the completed text
                                        edit_bytes.extend_from_slice(text.as_bytes());
                                        let _ = pty_tx.send(edit_bytes).await;

                                        // Update current_line
                                        for _ in 0..backspaces {
                                            current_line.pop();
                                        }
                                        current_line.push_str(&text);

                                        popup.dismiss();
                                        popup_lines = 0;
                                        popup_partial_len = 0;
                                        mode = Mode::Passthrough;
                                    }
                                }
                                InputAction::Escape | InputAction::CtrlC => {
                                    // Dismiss popup
                                    let mut clear_buf = Vec::new();
                                    let _ = renderer.clear(&mut clear_buf, popup_row, popup_col_actual, popup_lines);
                                    let _ = stdout_tx.send(clear_buf).await;
                                    popup.dismiss();
                                    popup_lines = 0;
                                    popup_partial_len = 0;
                                    mode = Mode::Passthrough;
                                    if action == InputAction::CtrlC {
                                        current_line.clear();
                                        let _ = pty_tx.send(vec![0x03]).await;
                                    }
                                }
                                InputAction::Backspace => {
                                    // Dismiss popup, pass through backspace
                                    let mut clear_buf = Vec::new();
                                    let _ = renderer.clear(&mut clear_buf, popup_row, popup_col_actual, popup_lines);
                                    let _ = stdout_tx.send(clear_buf).await;
                                    popup.dismiss();
                                    popup_lines = 0;
                                    popup_partial_len = 0;

                                    current_line.pop();
                                    let _ = pty_tx.send(vec![0x7f]).await;
                                    mode = Mode::Passthrough;
                                }
                                InputAction::KillWord => {
                                    let mut clear_buf = Vec::new();
                                    let _ = renderer.clear(&mut clear_buf, popup_row, popup_col_actual, popup_lines);
                                    let _ = stdout_tx.send(clear_buf).await;
                                    popup.dismiss();
                                    popup_lines = 0;
                                    popup_partial_len = 0;

                                    kill_last_word(&mut current_line);
                                    let _ = pty_tx.send(raw[offset - consumed..offset].to_vec()).await;
                                    mode = Mode::Passthrough;
                                }
                                InputAction::KillLine => {
                                    let mut clear_buf = Vec::new();
                                    let _ = renderer.clear(&mut clear_buf, popup_row, popup_col_actual, popup_lines);
                                    let _ = stdout_tx.send(clear_buf).await;
                                    popup.dismiss();
                                    popup_lines = 0;
                                    popup_partial_len = 0;

                                    current_line.clear();
                                    let _ = pty_tx.send(raw[offset - consumed..offset].to_vec()).await;
                                    mode = Mode::Passthrough;
                                }
                                InputAction::Passthrough => {
                                    track_passthrough_in_popup(&mut current_line, bytes);
                                    passthrough_buf.extend_from_slice(bytes);
                                }
                                _ => {
                                    // Dismiss on any other action
                                    let mut clear_buf = Vec::new();
                                    let _ = renderer.clear(&mut clear_buf, popup_row, popup_col_actual, popup_lines);
                                    let _ = stdout_tx.send(clear_buf).await;
                                    popup.dismiss();
                                    popup_lines = 0;
                                    popup_partial_len = 0;
                                    mode = Mode::Passthrough;
                                    let _ = pty_tx.send(raw[offset - consumed..offset].to_vec()).await;
                                }
                            }
                        }
                    }
                }

                if !passthrough_buf.is_empty() {
                    flush_passthrough_buffer(&pty_tx, &mut passthrough_buf).await;
                    if mode == Mode::PopupActive {
                        refresh_popup_completion(&mut engine, &mut matcher, &current_line, PopupRefreshState {
                            renderer: &renderer,
                            popup: &mut popup,
                            stdout_tx: &stdout_tx,
                            popup_row,
                            popup_col,
                            popup_col_actual: &mut popup_col_actual,
                            popup_lines: &mut popup_lines,
                            popup_partial_len: &mut popup_partial_len,
                            mode: &mut mode,
                        })
                        .await;
                    }
                }
            }

            // PTY output: track cursor position, then forward to stdout
            Some(data) = pty_out_rx.recv() => {
                let (_, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
                let mut i = 0;
                while i < data.len() {
                    match data[i] {
                        b'\n' => {
                            // Newline moves cursor down; at the bottom the terminal scrolls
                            // but the cursor stays on the last row.
                            if last_cursor_row < term_rows.saturating_sub(1) {
                                last_cursor_row += 1;
                            }
                            i += 1;
                        }
                        b'\r' => {
                            last_cursor_col = 0;
                            i += 1;
                        }
                        0x1b if i + 1 < data.len() && data[i + 1] == b'[' => {
                            // Parse CSI sequences for cursor position tracking.
                            // Handle \x1b[H (cursor home) and \x1b[row;colH (cursor position).
                            let seq_start = i + 2;
                            let mut j = seq_start;
                            while j < data.len() && (data[j].is_ascii_digit() || data[j] == b';') {
                                j += 1;
                            }
                            if j < data.len() && data[j] == b'H' {
                                // Cursor position command
                                let params = std::str::from_utf8(&data[seq_start..j]).unwrap_or("");
                                if params.is_empty() {
                                    // \x1b[H — home position
                                    last_cursor_row = 0;
                                    last_cursor_col = 0;
                                } else if let Some((row_str, col_str)) = params.split_once(';') {
                                    let row: u16 = row_str.parse().unwrap_or(1);
                                    let col: u16 = col_str.parse().unwrap_or(1);
                                    last_cursor_row = row.saturating_sub(1);
                                    last_cursor_col = col.saturating_sub(1);
                                }
                                i = j + 1;
                            } else {
                                i += 1;
                            }
                        }
                        _ => {
                            i += 1;
                        }
                    }
                }
                let _ = stdout_tx.send(data).await;
            }

            // Child process exited
            Some(_success) = child_rx.recv() => {
                child_exited = true;
                // Clean up popup if active
                if mode == Mode::PopupActive {
                    let mut clear_buf = Vec::new();
                    let _ = renderer.clear(&mut clear_buf, popup_row, popup_col_actual, popup_lines);
                    let _ = stdout_tx.send(clear_buf).await;
                }
                // Let remaining PTY output drain
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                break;
            }
        }

        if child_exited {
            break;
        }
    }

    // Cleanup
    shutdown.notify_waiters();
    drop(stdout_tx); // Close stdout channel
    drop(pty_tx); // Close PTY write channel
    sigwinch_handle.abort();
    stdin_handle.abort();
    pty_read_handle.abort();
    pty_write_handle.abort();
    stdout_write_handle.abort();
    child_wait_handle.abort();

    // Restore terminal
    let _ = crossterm::terminal::disable_raw_mode();

    Ok(if child_exited { 0 } else { 1 })
}
