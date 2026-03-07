use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};

use crate::completion::engine::CompletionEngine;
use crate::completion::loader::SpecStore;
use crate::completion::matcher::FuzzyMatcher;
use crate::completion::spec::CandidateKind;
use crate::config::Config;
use crate::input::line::{CompletionEdit, CompletionText, LineState};
use crate::input::parser::completion_edit_context;
use crate::input::trigger::{classify_input, InputAction};
use crate::shell::detect::detect_shell;
use crate::shell::escape::escape_fallback_completion;
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

const DELETE_KEY_SEQUENCE: &[u8] = b"\x1b[3~";
const CURSOR_LEFT_SEQUENCE: &[u8] = b"\x1b[D";

fn drain_channel_batch(receiver: &mut mpsc::Receiver<Vec<u8>>, mut batch: Vec<u8>) -> Vec<u8> {
    while let Ok(next) = receiver.try_recv() {
        batch.extend_from_slice(&next);
    }
    batch
}

fn track_passthrough_insert(
    line_state: &mut LineState,
    bytes: &[u8],
    line_start_row: &mut u16,
    line_start_col: &mut u16,
    last_cursor_row: &mut u16,
    last_cursor_col: &mut u16,
) {
    if bytes.first() == Some(&0x1b) {
        return;
    }

    if line_state.buffer().is_empty() && line_state.cursor() == 0 {
        *line_start_row = *last_cursor_row;
        *line_start_col = *last_cursor_col;
    }

    line_state.insert_bytes(bytes);
    sync_cursor_from_line(
        line_state,
        *line_start_row,
        *line_start_col,
        last_cursor_row,
        last_cursor_col,
    );
}

fn cursor_position_from_line(
    line_state: &LineState,
    line_start_row: u16,
    line_start_col: u16,
) -> (u16, u16) {
    let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
    if term_cols == 0 {
        return (line_start_row, line_start_col);
    }

    let absolute_col = line_start_col as usize + line_state.before_cursor().chars().count();
    let row_limit = term_rows.saturating_sub(1) as usize;
    let row = (line_start_row as usize + absolute_col / term_cols as usize).min(row_limit) as u16;
    let col = (absolute_col % term_cols as usize) as u16;
    (row, col)
}

fn sync_cursor_from_line(
    line_state: &LineState,
    line_start_row: u16,
    line_start_col: u16,
    last_cursor_row: &mut u16,
    last_cursor_col: &mut u16,
) {
    let (row, col) = cursor_position_from_line(line_state, line_start_row, line_start_col);
    *last_cursor_row = row;
    *last_cursor_col = col;
}

fn apply_completion_edit(edit: &CompletionEdit) -> Vec<u8> {
    let mut bytes = Vec::new();
    for _ in 0..edit.delete_right {
        bytes.extend_from_slice(DELETE_KEY_SEQUENCE);
    }
    bytes.extend(std::iter::repeat_n(0x7f, edit.delete_left));
    bytes.extend_from_slice(edit.insert_text.as_bytes());
    for _ in 0..edit.move_left {
        bytes.extend_from_slice(CURSOR_LEFT_SEQUENCE);
    }
    bytes
}

fn parse_osc7_path(sequence: &[u8]) -> Option<String> {
    if !sequence.starts_with(b"]7;file://") {
        return None;
    }

    let body = &sequence[b"]7;file://".len()..];
    let terminator_len = if body.ends_with(b"\x1b\\") { 2 } else { 1 };
    let content = &body[..body.len().saturating_sub(terminator_len)];
    let path_start = content.iter().position(|byte| *byte == b'/')?;
    let raw_path = std::str::from_utf8(&content[path_start..]).ok()?;
    Some(percent_decode(raw_path))
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &input[index + 1..index + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                decoded.push(value);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8(decoded).unwrap_or_else(|_| input.to_string())
}

fn utf8_char_len(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    }
}

fn advance_cursor(
    last_cursor_row: &mut u16,
    last_cursor_col: &mut u16,
    width: u16,
    term_cols: u16,
    term_rows: u16,
) {
    if term_cols == 0 || width == 0 {
        return;
    }

    for _ in 0..width {
        if last_cursor_col.saturating_add(1) >= term_cols {
            *last_cursor_col = 0;
            if *last_cursor_row < term_rows.saturating_sub(1) {
                *last_cursor_row += 1;
            }
        } else {
            *last_cursor_col += 1;
        }
    }
}

fn parse_csi_param(params: &str, index: usize, default: u16) -> u16 {
    params
        .trim_start_matches('?')
        .split(';')
        .nth(index)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn handle_csi_sequence(
    params: &str,
    final_byte: u8,
    term_rows: u16,
    term_cols: u16,
    last_cursor_row: &mut u16,
    last_cursor_col: &mut u16,
) {
    match final_byte {
        b'A' => {
            *last_cursor_row = last_cursor_row.saturating_sub(parse_csi_param(params, 0, 1));
        }
        b'B' => {
            *last_cursor_row = last_cursor_row
                .saturating_add(parse_csi_param(params, 0, 1))
                .min(term_rows.saturating_sub(1));
        }
        b'C' => {
            *last_cursor_col = last_cursor_col
                .saturating_add(parse_csi_param(params, 0, 1))
                .min(term_cols.saturating_sub(1));
        }
        b'D' => {
            *last_cursor_col = last_cursor_col.saturating_sub(parse_csi_param(params, 0, 1));
        }
        b'G' => {
            *last_cursor_col = parse_csi_param(params, 0, 1)
                .saturating_sub(1)
                .min(term_cols.saturating_sub(1));
        }
        b'H' | b'f' => {
            *last_cursor_row = parse_csi_param(params, 0, 1)
                .saturating_sub(1)
                .min(term_rows.saturating_sub(1));
            *last_cursor_col = parse_csi_param(params, 1, 1)
                .saturating_sub(1)
                .min(term_cols.saturating_sub(1));
        }
        _ => {}
    }
}

fn track_pty_output(
    data: &[u8],
    current_cwd: &mut String,
    osc_capture: &mut Option<Vec<u8>>,
    last_cursor_row: &mut u16,
    last_cursor_col: &mut u16,
) {
    let (term_cols, term_rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut index = 0;

    while index < data.len() {
        if let Some(sequence) = osc_capture.as_mut() {
            sequence.push(data[index]);
            let is_complete = data[index] == 0x07 || sequence.ends_with(b"\x1b\\");
            index += 1;
            if is_complete {
                if let Some(path) = parse_osc7_path(sequence) {
                    *current_cwd = path;
                }
                *osc_capture = None;
            }
            continue;
        }

        match data[index] {
            b'\n' => {
                if *last_cursor_row < term_rows.saturating_sub(1) {
                    *last_cursor_row += 1;
                }
                index += 1;
            }
            b'\r' => {
                *last_cursor_col = 0;
                index += 1;
            }
            0x08 => {
                *last_cursor_col = last_cursor_col.saturating_sub(1);
                index += 1;
            }
            b'\t' => {
                let tab_width = 8 - (*last_cursor_col % 8);
                advance_cursor(
                    last_cursor_row,
                    last_cursor_col,
                    tab_width.max(1),
                    term_cols,
                    term_rows,
                );
                index += 1;
            }
            0x1b if index + 1 < data.len() && data[index + 1] == b']' => {
                *osc_capture = Some(vec![b']']);
                index += 2;
            }
            0x1b if index + 1 < data.len() && data[index + 1] == b'[' => {
                let seq_start = index + 2;
                let mut seq_end = seq_start;
                while seq_end < data.len() && !(0x40..=0x7e).contains(&data[seq_end]) {
                    seq_end += 1;
                }
                if seq_end >= data.len() {
                    break;
                }

                let params = std::str::from_utf8(&data[seq_start..seq_end]).unwrap_or("");
                handle_csi_sequence(
                    params,
                    data[seq_end],
                    term_rows,
                    term_cols,
                    last_cursor_row,
                    last_cursor_col,
                );
                index = seq_end + 1;
            }
            byte if byte.is_ascii_control() => {
                index += 1;
            }
            byte if byte.is_ascii() => {
                advance_cursor(last_cursor_row, last_cursor_col, 1, term_cols, term_rows);
                index += 1;
            }
            byte if (byte & 0b1100_0000) != 0b1000_0000 => {
                advance_cursor(last_cursor_row, last_cursor_col, 1, term_cols, term_rows);
                index += utf8_char_len(byte).min(data.len() - index);
            }
            _ => {
                index += 1;
            }
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

struct PopupCloseState<'a> {
    renderer: &'a PopupRenderer,
    popup: &'a mut PopupState,
    stdout_tx: &'a mpsc::Sender<Vec<u8>>,
    popup_row: u16,
    popup_col_actual: u16,
    popup_lines: &'a mut u16,
    popup_partial_len: &'a mut usize,
    mode: &'a mut Mode,
}

async fn refresh_popup_completion(
    engine: &mut CompletionEngine,
    matcher: &mut FuzzyMatcher,
    line_state: &LineState,
    cwd: &str,
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

    let completion = engine.complete(line_state.before_cursor(), cwd);
    let scored = matcher.filter(&completion.partial, completion.candidates);

    if !scored.is_empty() {
        *popup_partial_len = completion.partial.chars().count();
        let mut render_buf = Vec::new();
        let _ = renderer.clear(&mut render_buf, popup_row, *popup_col_actual, *popup_lines);
        popup.set_items_preserve_selection(scored);
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

async fn close_popup(state: PopupCloseState<'_>) {
    let PopupCloseState {
        renderer,
        popup,
        stdout_tx,
        popup_row,
        popup_col_actual,
        popup_lines,
        popup_partial_len,
        mode,
    } = state;
    let mut clear_buf = Vec::new();
    let _ = renderer.clear(&mut clear_buf, popup_row, popup_col_actual, *popup_lines);
    let _ = stdout_tx.send(clear_buf).await;
    popup.dismiss();
    *popup_lines = 0;
    *popup_partial_len = 0;
    *mode = Mode::Passthrough;
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
    let mut line_state = LineState::default();
    let mut current_cwd = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let mut popup_lines: u16 = 0;
    let mut popup_partial_len: usize = 0;
    // Cursor tracking: initialised from DSR query, updated via PTY output newlines.
    let mut last_cursor_row: u16 = init_row;
    let mut last_cursor_col: u16 = init_col;
    let mut line_start_row: u16 = init_row;
    let mut line_start_col: u16 = init_col;
    // Snapshot of cursor position when the popup was opened (used for render/clear).
    let mut popup_row: u16 = 0;
    let mut popup_col: u16 = 0;
    // Actual column render() placed the popup at (may differ from popup_col when near right edge).
    let mut popup_col_actual: u16 = 0;
    let mut osc_capture: Option<Vec<u8>> = None;
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
                            popup_row = last_cursor_row;
                            popup_col = last_cursor_col;
                            refresh_popup_completion(
                                &mut engine,
                                &mut matcher,
                                &line_state,
                                &current_cwd,
                                PopupRefreshState {
                                    renderer: &renderer,
                                    popup: &mut popup,
                                    stdout_tx: &stdout_tx,
                                    popup_row,
                                    popup_col,
                                    popup_col_actual: &mut popup_col_actual,
                                    popup_lines: &mut popup_lines,
                                    popup_partial_len: &mut popup_partial_len,
                                    mode: &mut mode,
                                },
                            )
                            .await;
                        }
                    }

                    match mode {
                        Mode::Passthrough => match action {
                            InputAction::Tab => {
                                if !line_state.before_cursor().trim().is_empty() {
                                    let completion =
                                        engine.complete(line_state.before_cursor(), &current_cwd);
                                    let scored =
                                        matcher.filter(&completion.partial, completion.candidates);
                                    if !scored.is_empty() {
                                        popup_partial_len = completion.partial.chars().count();
                                        popup.set_items(scored);
                                        mode = Mode::PopupActive;
                                        popup_row = last_cursor_row;
                                        popup_col = last_cursor_col;
                                        let mut render_buf = Vec::new();
                                        let (lines, col) = renderer
                                            .render(&mut render_buf, &popup, popup_row, popup_col)
                                            .unwrap_or((0, popup_col));
                                        popup_lines = lines;
                                        popup_col_actual = col;
                                        let _ = stdout_tx.send(render_buf).await;
                                    } else {
                                        popup_partial_len = 0;
                                        let _ = pty_tx.send(vec![0x09]).await;
                                    }
                                } else {
                                    let _ = pty_tx.send(vec![0x09]).await;
                                }
                            }
                            InputAction::Passthrough => {
                                track_passthrough_insert(
                                    &mut line_state,
                                    bytes,
                                    &mut line_start_row,
                                    &mut line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                passthrough_buf.extend_from_slice(bytes);
                            }
                            InputAction::Enter => {
                                line_state.clear();
                                popup_partial_len = 0;
                                let _ = pty_tx.send(vec![0x0d]).await;
                            }
                            InputAction::Backspace => {
                                let _ = line_state.backspace();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(vec![0x7f]).await;
                            }
                            InputAction::Delete => {
                                let _ = line_state.delete();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(bytes.to_vec()).await;
                            }
                            InputAction::Left => {
                                let _ = line_state.move_left();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(bytes.to_vec()).await;
                            }
                            InputAction::Right => {
                                let _ = line_state.move_right();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(bytes.to_vec()).await;
                            }
                            InputAction::Home => {
                                let _ = line_state.move_home();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(bytes.to_vec()).await;
                            }
                            InputAction::End => {
                                let _ = line_state.move_end();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(bytes.to_vec()).await;
                            }
                            InputAction::CtrlC => {
                                line_state.clear();
                                popup_partial_len = 0;
                                let _ = pty_tx.send(vec![0x03]).await;
                            }
                            InputAction::CtrlZ => {
                                line_state.clear();
                                popup_partial_len = 0;
                                let _ = pty_tx.send(vec![0x1a]).await;
                            }
                            InputAction::CtrlJ => {
                                let _ = pty_tx.send(vec![0x0a]).await;
                            }
                            InputAction::CtrlK => {
                                let _ = pty_tx.send(vec![0x0b]).await;
                            }
                            InputAction::KillWord => {
                                let _ = line_state.kill_last_word();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(bytes.to_vec()).await;
                            }
                            InputAction::KillLine => {
                                let _ = line_state.kill_line();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(bytes.to_vec()).await;
                            }
                            InputAction::Up | InputAction::Down => {
                                line_state.clear();
                                popup_partial_len = 0;
                                let _ = pty_tx.send(bytes.to_vec()).await;
                            }
                            _ => {
                                let _ = pty_tx.send(bytes.to_vec()).await;
                            }
                        },
                        Mode::PopupActive => match action {
                            InputAction::Down | InputAction::Tab | InputAction::CtrlJ => {
                                popup.select_next();
                                let mut render_buf = Vec::new();
                                let (lines, col) = renderer
                                    .render(&mut render_buf, &popup, popup_row, popup_col)
                                    .unwrap_or((0, popup_col_actual));
                                popup_lines = lines;
                                popup_col_actual = col;
                                let _ = stdout_tx.send(render_buf).await;
                            }
                            InputAction::Up | InputAction::ShiftTab | InputAction::CtrlK => {
                                popup.select_prev();
                                let mut render_buf = Vec::new();
                                let (lines, col) = renderer
                                    .render(&mut render_buf, &popup, popup_row, popup_col)
                                    .unwrap_or((0, popup_col_actual));
                                popup_lines = lines;
                                popup_col_actual = col;
                                let _ = stdout_tx.send(render_buf).await;
                            }
                            InputAction::Enter => {
                                if let Some(candidate) =
                                    popup.selected_item().map(|item| item.candidate.clone())
                                {
                                    let edit_context =
                                        completion_edit_context(line_state.buffer(), line_state.cursor());
                                    let raw_insert = if candidate.insert_value.is_some() {
                                        candidate.insert_text().to_string()
                                    } else {
                                        escape_fallback_completion(
                                            &shell_type,
                                            edit_context.quote_mode,
                                            candidate.insert_text(),
                                        )
                                    };
                                    let completion = CompletionText::from_insert_value(&raw_insert);
                                    let append_space = line_state.should_append_space_for_span(
                                        matches!(candidate.kind, CandidateKind::Folder),
                                        edit_context.replacement_end,
                                        edit_context.quote_mode,
                                    ) && completion.cursor_at_end()
                                        && !completion.submits_line;
                                    let edit = line_state.apply_completion_span(
                                        &completion,
                                        edit_context.replacement_start,
                                        edit_context.replacement_end,
                                        append_space,
                                    );
                                    sync_cursor_from_line(
                                        &line_state,
                                        line_start_row,
                                        line_start_col,
                                        &mut last_cursor_row,
                                        &mut last_cursor_col,
                                    );
                                    close_popup(PopupCloseState {
                                        renderer: &renderer,
                                        popup: &mut popup,
                                        stdout_tx: &stdout_tx,
                                        popup_row,
                                        popup_col_actual,
                                        popup_lines: &mut popup_lines,
                                        popup_partial_len: &mut popup_partial_len,
                                        mode: &mut mode,
                                    })
                                    .await;
                                    let _ = pty_tx.send(apply_completion_edit(&edit)).await;
                                }
                            }
                            InputAction::Escape | InputAction::CtrlC => {
                                close_popup(PopupCloseState {
                                    renderer: &renderer,
                                    popup: &mut popup,
                                    stdout_tx: &stdout_tx,
                                    popup_row,
                                    popup_col_actual,
                                    popup_lines: &mut popup_lines,
                                    popup_partial_len: &mut popup_partial_len,
                                    mode: &mut mode,
                                })
                                .await;
                                if action == InputAction::CtrlC {
                                    line_state.clear();
                                    let _ = pty_tx.send(vec![0x03]).await;
                                }
                            }
                            InputAction::Backspace => {
                                let _ = line_state.backspace();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(vec![0x7f]).await;
                                popup_row = last_cursor_row;
                                popup_col = last_cursor_col;
                                refresh_popup_completion(
                                    &mut engine,
                                    &mut matcher,
                                    &line_state,
                                    &current_cwd,
                                    PopupRefreshState {
                                        renderer: &renderer,
                                        popup: &mut popup,
                                        stdout_tx: &stdout_tx,
                                        popup_row,
                                        popup_col,
                                        popup_col_actual: &mut popup_col_actual,
                                        popup_lines: &mut popup_lines,
                                        popup_partial_len: &mut popup_partial_len,
                                        mode: &mut mode,
                                    },
                                )
                                .await;
                            }
                            InputAction::Delete => {
                                let _ = line_state.delete();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(bytes.to_vec()).await;
                                popup_row = last_cursor_row;
                                popup_col = last_cursor_col;
                                refresh_popup_completion(
                                    &mut engine,
                                    &mut matcher,
                                    &line_state,
                                    &current_cwd,
                                    PopupRefreshState {
                                        renderer: &renderer,
                                        popup: &mut popup,
                                        stdout_tx: &stdout_tx,
                                        popup_row,
                                        popup_col,
                                        popup_col_actual: &mut popup_col_actual,
                                        popup_lines: &mut popup_lines,
                                        popup_partial_len: &mut popup_partial_len,
                                        mode: &mut mode,
                                    },
                                )
                                .await;
                            }
                            InputAction::Left
                            | InputAction::Right
                            | InputAction::Home
                            | InputAction::End => {
                                match action {
                                    InputAction::Left => {
                                        let _ = line_state.move_left();
                                    }
                                    InputAction::Right => {
                                        let _ = line_state.move_right();
                                    }
                                    InputAction::Home => {
                                        let _ = line_state.move_home();
                                    }
                                    InputAction::End => {
                                        let _ = line_state.move_end();
                                    }
                                    _ => {}
                                }
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(bytes.to_vec()).await;
                                popup_row = last_cursor_row;
                                popup_col = last_cursor_col;
                                refresh_popup_completion(
                                    &mut engine,
                                    &mut matcher,
                                    &line_state,
                                    &current_cwd,
                                    PopupRefreshState {
                                        renderer: &renderer,
                                        popup: &mut popup,
                                        stdout_tx: &stdout_tx,
                                        popup_row,
                                        popup_col,
                                        popup_col_actual: &mut popup_col_actual,
                                        popup_lines: &mut popup_lines,
                                        popup_partial_len: &mut popup_partial_len,
                                        mode: &mut mode,
                                    },
                                )
                                .await;
                            }
                            InputAction::KillWord => {
                                let _ = line_state.kill_last_word();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(bytes.to_vec()).await;
                                popup_row = last_cursor_row;
                                popup_col = last_cursor_col;
                                refresh_popup_completion(
                                    &mut engine,
                                    &mut matcher,
                                    &line_state,
                                    &current_cwd,
                                    PopupRefreshState {
                                        renderer: &renderer,
                                        popup: &mut popup,
                                        stdout_tx: &stdout_tx,
                                        popup_row,
                                        popup_col,
                                        popup_col_actual: &mut popup_col_actual,
                                        popup_lines: &mut popup_lines,
                                        popup_partial_len: &mut popup_partial_len,
                                        mode: &mut mode,
                                    },
                                )
                                .await;
                            }
                            InputAction::KillLine => {
                                let _ = line_state.kill_line();
                                sync_cursor_from_line(
                                    &line_state,
                                    line_start_row,
                                    line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                let _ = pty_tx.send(bytes.to_vec()).await;
                                popup_row = last_cursor_row;
                                popup_col = last_cursor_col;
                                refresh_popup_completion(
                                    &mut engine,
                                    &mut matcher,
                                    &line_state,
                                    &current_cwd,
                                    PopupRefreshState {
                                        renderer: &renderer,
                                        popup: &mut popup,
                                        stdout_tx: &stdout_tx,
                                        popup_row,
                                        popup_col,
                                        popup_col_actual: &mut popup_col_actual,
                                        popup_lines: &mut popup_lines,
                                        popup_partial_len: &mut popup_partial_len,
                                        mode: &mut mode,
                                    },
                                )
                                .await;
                            }
                            InputAction::Passthrough => {
                                track_passthrough_insert(
                                    &mut line_state,
                                    bytes,
                                    &mut line_start_row,
                                    &mut line_start_col,
                                    &mut last_cursor_row,
                                    &mut last_cursor_col,
                                );
                                passthrough_buf.extend_from_slice(bytes);
                            }
                            _ => {
                                close_popup(PopupCloseState {
                                    renderer: &renderer,
                                    popup: &mut popup,
                                    stdout_tx: &stdout_tx,
                                    popup_row,
                                    popup_col_actual,
                                    popup_lines: &mut popup_lines,
                                    popup_partial_len: &mut popup_partial_len,
                                    mode: &mut mode,
                                })
                                .await;
                                let _ = pty_tx.send(bytes.to_vec()).await;
                            }
                        },
                    }
                }

                if !passthrough_buf.is_empty() {
                    flush_passthrough_buffer(&pty_tx, &mut passthrough_buf).await;
                    if mode == Mode::PopupActive {
                        popup_row = last_cursor_row;
                        popup_col = last_cursor_col;
                        refresh_popup_completion(
                            &mut engine,
                            &mut matcher,
                            &line_state,
                            &current_cwd,
                            PopupRefreshState {
                                renderer: &renderer,
                                popup: &mut popup,
                                stdout_tx: &stdout_tx,
                                popup_row,
                                popup_col,
                                popup_col_actual: &mut popup_col_actual,
                                popup_lines: &mut popup_lines,
                                popup_partial_len: &mut popup_partial_len,
                                mode: &mut mode,
                            },
                        )
                        .await;
                    }
                }
            }

            // PTY output: track cursor position, then forward to stdout
            Some(data) = pty_out_rx.recv() => {
                track_pty_output(
                    &data,
                    &mut current_cwd,
                    &mut osc_capture,
                    &mut last_cursor_row,
                    &mut last_cursor_col,
                );
                let _ = stdout_tx.send(data).await;
            }

            // Child process exited
            Some(_success) = child_rx.recv() => {
                child_exited = true;
                // Clean up popup if active
                if mode == Mode::PopupActive {
                    close_popup(PopupCloseState {
                        renderer: &renderer,
                        popup: &mut popup,
                        stdout_tx: &stdout_tx,
                        popup_row,
                        popup_col_actual,
                        popup_lines: &mut popup_lines,
                        popup_partial_len: &mut popup_partial_len,
                        mode: &mut mode,
                    })
                    .await;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_osc7_path_decodes_percent_encoding() {
        assert_eq!(
            parse_osc7_path(b"]7;file://localhost/tmp/my%20project\x07"),
            Some("/tmp/my project".into())
        );
        assert_eq!(
            parse_osc7_path(b"]7;file://localhost/tmp/my%20project\x1b\\"),
            Some("/tmp/my project".into())
        );
    }

    #[test]
    fn test_track_pty_output_updates_cwd_and_cursor() {
        let mut cwd = ".".to_string();
        let mut osc_capture = None;
        let mut row = 0;
        let mut col = 0;

        track_pty_output(b"abc", &mut cwd, &mut osc_capture, &mut row, &mut col);
        assert_eq!((row, col), (0, 3));

        track_pty_output(
            b"\x1b[10G\x1b[2D",
            &mut cwd,
            &mut osc_capture,
            &mut row,
            &mut col,
        );
        assert_eq!(col, 7);

        track_pty_output(
            b"\x1b]7;file://localhost/tmp/project\x07",
            &mut cwd,
            &mut osc_capture,
            &mut row,
            &mut col,
        );
        assert_eq!(cwd, "/tmp/project");
    }
}
