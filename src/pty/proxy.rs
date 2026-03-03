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

    let engine = CompletionEngine::new(store);
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
    cmd.env("TERM", std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_string()));
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
    let mut pty_writer = master
        .take_writer()
        .context("failed to take PTY writer")?;

    // Put host terminal into raw mode
    crossterm::terminal::enable_raw_mode().context("failed to enable raw mode")?;

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
            if pty_writer.write_all(&data).is_err() {
                break;
            }
            let _ = pty_writer.flush();
        }
    });

    // Task: PTY reader → stdout (with popup overlay support)
    // We use a channel so the main loop can also write to stdout (for popup rendering)
    let (stdout_tx, mut stdout_rx) = mpsc::channel::<Vec<u8>>(64);
    let stdout_tx_pty = stdout_tx.clone();

    let shutdown_stdout = shutdown.clone();
    let pty_read_handle = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 4096];
        loop {
            match pty_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if stdout_tx_pty.blocking_send(buf[..n].to_vec()).is_err() {
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
            let _ = stdout.write_all(&data);
            let _ = stdout.flush();
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
    let theme = Theme::default();
    let max_visible = config.max_visible.unwrap_or(theme.max_visible);
    let renderer = PopupRenderer::new(theme);
    let mut popup = PopupState::new(max_visible);
    let mut mode = Mode::Passthrough;
    let mut current_line = String::new();
    let mut popup_lines: u16 = 0;
    let mut last_cursor_row: u16 = 0;
    let mut last_cursor_col: u16 = 0;
    let mut child_exited = false;

    loop {
        tokio::select! {
            // Input from stdin
            Some(raw) = stdin_rx.recv() => {
                let mut offset = 0;
                while offset < raw.len() {
                    let (action, consumed) = classify_input(&raw[offset..]);
                    if consumed == 0 { break; }
                    offset += consumed;

                    match mode {
                        Mode::Passthrough => {
                            match action {
                                InputAction::Tab => {
                                    // Trigger completion
                                    if !current_line.trim().is_empty() {
                                        let (_, partial) = crate::input::parser::split_partial(&current_line);
                                        let candidates = engine.complete(&current_line);
                                        let scored = matcher.filter(&partial, candidates);
                                        if !scored.is_empty() {
                                            popup.set_items(scored);
                                            mode = Mode::PopupActive;
                                            // Get cursor position for rendering
                                            let (_, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
                                            // Use DSR (Device Status Report) to get cursor position
                                            // For now, estimate from terminal size
                                            last_cursor_row = term_rows.saturating_sub(1);
                                            last_cursor_col = current_line.len() as u16;
                                            // Render popup
                                            let mut render_buf = Vec::new();
                                            popup_lines = renderer.render(&mut render_buf, &popup, last_cursor_row, last_cursor_col).unwrap_or(0);
                                            let _ = stdout_tx.send(render_buf).await;
                                        } else {
                                            // No completions — pass tab through
                                            let _ = pty_tx.send(vec![0x09]).await;
                                        }
                                    } else {
                                        let _ = pty_tx.send(vec![0x09]).await;
                                    }
                                }
                                InputAction::Passthrough(bytes) => {
                                    // Track the current line
                                    for &b in &bytes {
                                        if b == b'\r' || b == b'\n' {
                                            current_line.clear();
                                        } else if b == 0x7f || b == 0x08 {
                                            current_line.pop();
                                        } else if b >= 0x20 {
                                            current_line.push(b as char);
                                        }
                                    }
                                    let _ = pty_tx.send(bytes).await;
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
                                _ => {
                                    // Pass through everything else
                                    let _ = pty_tx.send(raw[offset - consumed..offset].to_vec()).await;
                                }
                            }
                        }
                        Mode::PopupActive => {
                            match action {
                                InputAction::Down | InputAction::Tab => {
                                    popup.select_next();
                                    let mut render_buf = Vec::new();
                                    popup_lines = renderer.render(&mut render_buf, &popup, last_cursor_row, last_cursor_col).unwrap_or(0);
                                    let _ = stdout_tx.send(render_buf).await;
                                }
                                InputAction::Up | InputAction::ShiftTab => {
                                    popup.select_prev();
                                    let mut render_buf = Vec::new();
                                    popup_lines = renderer.render(&mut render_buf, &popup, last_cursor_row, last_cursor_col).unwrap_or(0);
                                    let _ = stdout_tx.send(render_buf).await;
                                }
                                InputAction::Enter => {
                                    // Accept the selected completion
                                    if let Some(text) = popup.selected_text() {
                                        let text = text.to_string();
                                        // Clear popup first
                                        let mut clear_buf = Vec::new();
                                        let _ = renderer.clear(&mut clear_buf, last_cursor_row, last_cursor_col, popup_lines, 80);
                                        let _ = stdout_tx.send(clear_buf).await;

                                        // Calculate how much to backspace (remove partial)
                                        let (_, partial) = crate::input::parser::split_partial(&current_line);
                                        let backspaces = partial.len();

                                        // Send backspaces to delete partial
                                        let mut edit_bytes = Vec::new();
                                        for _ in 0..backspaces {
                                            edit_bytes.push(0x7f);
                                        }
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
                                        mode = Mode::Passthrough;
                                    }
                                }
                                InputAction::Escape | InputAction::CtrlC => {
                                    // Dismiss popup
                                    let mut clear_buf = Vec::new();
                                    let _ = renderer.clear(&mut clear_buf, last_cursor_row, last_cursor_col, popup_lines, 80);
                                    let _ = stdout_tx.send(clear_buf).await;
                                    popup.dismiss();
                                    popup_lines = 0;
                                    mode = Mode::Passthrough;
                                    if action == InputAction::CtrlC {
                                        current_line.clear();
                                        let _ = pty_tx.send(vec![0x03]).await;
                                    }
                                }
                                InputAction::Backspace => {
                                    // Dismiss popup, pass through backspace, retrigger
                                    let mut clear_buf = Vec::new();
                                    let _ = renderer.clear(&mut clear_buf, last_cursor_row, last_cursor_col, popup_lines, 80);
                                    let _ = stdout_tx.send(clear_buf).await;
                                    popup.dismiss();
                                    popup_lines = 0;

                                    current_line.pop();
                                    let _ = pty_tx.send(vec![0x7f]).await;
                                    mode = Mode::Passthrough;
                                }
                                InputAction::Passthrough(bytes) => {
                                    // Typing while popup is open — dismiss and pass through
                                    let mut clear_buf = Vec::new();
                                    let _ = renderer.clear(&mut clear_buf, last_cursor_row, last_cursor_col, popup_lines, 80);
                                    let _ = stdout_tx.send(clear_buf).await;
                                    popup.dismiss();
                                    popup_lines = 0;

                                    for &b in &bytes {
                                        if b >= 0x20 {
                                            current_line.push(b as char);
                                        }
                                    }
                                    let _ = pty_tx.send(bytes).await;
                                    mode = Mode::Passthrough;
                                }
                                _ => {
                                    // Dismiss on any other action
                                    let mut clear_buf = Vec::new();
                                    let _ = renderer.clear(&mut clear_buf, last_cursor_row, last_cursor_col, popup_lines, 80);
                                    let _ = stdout_tx.send(clear_buf).await;
                                    popup.dismiss();
                                    popup_lines = 0;
                                    mode = Mode::Passthrough;
                                    let _ = pty_tx.send(raw[offset - consumed..offset].to_vec()).await;
                                }
                            }
                        }
                    }
                }
            }

            // Child process exited
            Some(_success) = child_rx.recv() => {
                child_exited = true;
                // Clean up popup if active
                if mode == Mode::PopupActive {
                    let mut clear_buf = Vec::new();
                    let _ = renderer.clear(&mut clear_buf, last_cursor_row, last_cursor_col, popup_lines, 80);
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
    drop(pty_tx);    // Close PTY write channel
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
