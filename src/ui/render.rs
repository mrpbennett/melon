use crossterm::cursor;
use crossterm::style;
use crossterm::terminal;
use std::io::Write;
use unicode_width::UnicodeWidthStr;

use super::popup::PopupState;
use super::theme::{border, Theme};

/// Wrap `text` into lines of at most `max_width` display columns (splits on spaces).
fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;

    for word in text.split_whitespace() {
        let word_w = UnicodeWidthStr::width(word);
        if current_w == 0 {
            current.push_str(word);
            current_w = word_w;
        } else if current_w + 1 + word_w <= max_width {
            current.push(' ');
            current.push_str(word);
            current_w += 1 + word_w;
        } else {
            lines.push(current.clone());
            current = word.to_string();
            current_w = word_w;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Render the autocomplete popup to the terminal.
/// The popup is drawn below the cursor position.
pub struct PopupRenderer {
    theme: Theme,
}

impl PopupRenderer {
    pub fn new(theme: Theme) -> Self {
        Self { theme }
    }

    /// Render the popup at the given cursor position (row, col).
    /// Returns `(lines_drawn, actual_popup_col)` for use by `clear()`.
    pub fn render(
        &self,
        stdout: &mut impl Write,
        state: &PopupState,
        cursor_row: u16,
        cursor_col: u16,
    ) -> std::io::Result<(u16, u16)> {
        if !state.visible || state.items.is_empty() {
            return Ok((0, cursor_col));
        }

        let (term_cols, term_rows) = terminal::size().unwrap_or((80, 24));
        let visible_count = state.visible_count();

        // Calculate popup dimensions
        let mut max_name_width = 0usize;
        let mut max_desc_width = 0usize;
        for i in state.scroll_offset..state.scroll_offset + visible_count {
            if let Some(item) = state.items.get(i) {
                let name_w = UnicodeWidthStr::width(item.candidate.name.as_str());
                max_name_width = max_name_width.max(name_w);
                if let Some(desc) = &item.candidate.description {
                    max_desc_width = max_desc_width.max(UnicodeWidthStr::width(desc.as_str()));
                }
            }
        }

        // Content width: icon(2) + name + gap(2) + description
        let has_descriptions = max_desc_width > 0;
        let content_width = if has_descriptions {
            2 + max_name_width + 2 + max_desc_width.min(30)
        } else {
            2 + max_name_width
        };
        let inner_width = content_width.clamp(self.theme.min_width, self.theme.max_width);
        let popup_width = inner_width + 2; // borders

        // Position: below cursor, clamped to terminal bounds
        let popup_row = if cursor_row + 1 + visible_count as u16 + 2 > term_rows {
            // Not enough space below — draw above
            cursor_row.saturating_sub(visible_count as u16 + 2)
        } else {
            cursor_row + 1
        };
        let popup_col = if cursor_col + popup_width as u16 > term_cols {
            term_cols.saturating_sub(popup_width as u16)
        } else {
            cursor_col
        };

        // Save cursor position
        crossterm::execute!(stdout, cursor::SavePosition)?;

        // Draw top border
        crossterm::execute!(
            stdout,
            cursor::MoveTo(popup_col, popup_row),
            style::SetForegroundColor(self.theme.border),
            style::SetBackgroundColor(self.theme.bg),
            style::Print(border::TOP_LEFT),
            style::Print(border::HORIZONTAL.repeat(inner_width)),
            style::Print(border::TOP_RIGHT),
        )?;

        // Draw items
        for i in 0..visible_count {
            let idx = state.scroll_offset + i;
            let item = match state.items.get(idx) {
                Some(item) => item,
                None => break,
            };

            let is_selected = idx == state.selected;
            let bg = if is_selected { self.theme.selected_bg } else { self.theme.bg };
            let fg = if is_selected { self.theme.selected_fg } else { self.theme.fg };

            // Kind icon (muted, 2 chars wide)
            let icon = match item.candidate.kind {
                crate::completion::spec::CandidateKind::Subcommand => "❯ ",
                crate::completion::spec::CandidateKind::Option => "- ",
                crate::completion::spec::CandidateKind::Argument => "· ",
                crate::completion::spec::CandidateKind::File => "f ",
                crate::completion::spec::CandidateKind::Folder => "d ",
            };
            let icon_fg = if is_selected {
                self.theme.match_fg
            } else {
                self.theme.description_fg
            };

            // Left border: accent bar for selected, plain border otherwise
            let (left_border, left_border_fg) = if is_selected {
                (border::VERTICAL, self.theme.match_fg)
            } else {
                (border::VERTICAL, self.theme.border)
            };

            let name = &item.candidate.name;
            let name_width = UnicodeWidthStr::width(name.as_str());
            let name_padding = max_name_width.saturating_sub(name_width);

            let row = popup_row + 1 + i as u16;
            crossterm::execute!(
                stdout,
                cursor::MoveTo(popup_col, row),
                style::SetForegroundColor(left_border_fg),
                style::SetBackgroundColor(bg),
                style::Print(left_border),
                style::SetForegroundColor(icon_fg),
                style::Print(icon),
                style::SetForegroundColor(fg),
                style::Print(name),
                style::Print(" ".repeat(name_padding)),
            )?;

            // Description (if fits)
            // Reserve 3 for "  ·" separator prefix, 1 for trailing space
            let remaining = inner_width.saturating_sub(2 + name_width + name_padding);
            if has_descriptions && remaining > 5 {
                let desc = item.candidate.description.as_deref().unwrap_or("");
                // "  · " = 4 chars overhead before desc text
                let desc_max = remaining.saturating_sub(4);
                let truncated = if UnicodeWidthStr::width(desc) > desc_max {
                    let mut end = desc_max.min(desc.len());
                    while end > 0 && !desc.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}…", &desc[..end.saturating_sub(1)])
                } else {
                    desc.to_string()
                };
                let desc_width = UnicodeWidthStr::width(truncated.as_str());
                let desc_padding = remaining.saturating_sub(4 + desc_width);
                crossterm::execute!(
                    stdout,
                    style::SetForegroundColor(self.theme.border),
                    style::Print("  ·"),
                    style::SetForegroundColor(self.theme.description_fg),
                    style::Print(" "),
                    style::Print(&truncated),
                    style::Print(" ".repeat(desc_padding)),
                )?;
            } else {
                let pad = inner_width.saturating_sub(2 + name_width + name_padding);
                crossterm::execute!(stdout, style::Print(" ".repeat(pad)))?;
            }

            crossterm::execute!(
                stdout,
                style::SetForegroundColor(self.theme.border),
                style::SetBackgroundColor(bg),
                style::Print(border::VERTICAL),
            )?;
        }

        // Draw bottom border
        let bottom_row = popup_row + 1 + visible_count as u16;
        // Scroll indicator
        let total = state.items.len();
        let indicator = if total > state.max_visible {
            let pos = state.scroll_offset + visible_count;
            format!(" {pos}/{total} ↕ ")
        } else {
            String::new()
        };
        let indicator_width = UnicodeWidthStr::width(indicator.as_str());
        let border_remaining = inner_width.saturating_sub(indicator_width);

        crossterm::execute!(
            stdout,
            cursor::MoveTo(popup_col, bottom_row),
            style::SetForegroundColor(self.theme.border),
            style::SetBackgroundColor(self.theme.bg),
            style::Print(border::BOTTOM_LEFT),
            style::Print(border::HORIZONTAL.repeat(border_remaining)),
            style::SetForegroundColor(self.theme.description_fg),
            style::Print(&indicator),
            style::SetForegroundColor(self.theme.border),
            style::Print(border::BOTTOM_RIGHT),
        )?;

        // Description panel (optional)
        if self.theme.show_description_panel {
            if let Some(selected) = state.items.get(state.selected) {
                if let Some(desc) = &selected.candidate.description {
                    let name = &selected.candidate.name;
                    let panel_col = popup_col + popup_width as u16 + 1;
                    let panel_inner_width: usize = 36;
                    let panel_width = panel_inner_width + 2;

                    // Only draw if panel fits on screen
                    if panel_col + panel_width as u16 <= term_cols {
                        // Word-wrap description into lines of panel_inner_width
                        let wrap_width = panel_inner_width.saturating_sub(2); // 1-char padding each side
                        let desc_lines = word_wrap(desc, wrap_width);

                        // Panel height = max(visible_count + 2, 2 + 1 name line + desc lines + 1 padding)
                        let content_lines = 1 + desc_lines.len(); // name + desc
                        let panel_height = (visible_count + 2).max(content_lines + 3);

                        // Top border
                        crossterm::execute!(
                            stdout,
                            cursor::MoveTo(panel_col, popup_row),
                            style::SetForegroundColor(self.theme.border),
                            style::SetBackgroundColor(self.theme.bg),
                            style::Print(border::TOP_LEFT),
                            style::Print(border::HORIZONTAL.repeat(panel_inner_width)),
                            style::Print(border::TOP_RIGHT),
                        )?;

                        // Name line
                        let name_w = UnicodeWidthStr::width(name.as_str()).min(wrap_width);
                        let name_pad = wrap_width.saturating_sub(name_w);
                        crossterm::execute!(
                            stdout,
                            cursor::MoveTo(panel_col, popup_row + 1),
                            style::SetForegroundColor(self.theme.border),
                            style::SetBackgroundColor(self.theme.bg),
                            style::Print(border::VERTICAL),
                            style::SetForegroundColor(self.theme.selected_fg),
                            style::Print(" "),
                            style::Print(&name[..name.len().min(wrap_width)]),
                            style::Print(" ".repeat(name_pad + 1)),
                            style::SetForegroundColor(self.theme.border),
                            style::Print(border::VERTICAL),
                        )?;

                        // Separator
                        crossterm::execute!(
                            stdout,
                            cursor::MoveTo(panel_col, popup_row + 2),
                            style::SetForegroundColor(self.theme.border),
                            style::SetBackgroundColor(self.theme.bg),
                            style::Print(border::VERTICAL),
                            style::Print(border::HORIZONTAL.repeat(panel_inner_width)),
                            style::Print(border::VERTICAL),
                        )?;

                        // Description lines
                        for (i, line) in desc_lines.iter().enumerate() {
                            let row = popup_row + 3 + i as u16;
                            if row >= popup_row + panel_height as u16 - 1 {
                                break;
                            }
                            let line_w = UnicodeWidthStr::width(line.as_str());
                            let line_pad = wrap_width.saturating_sub(line_w);
                            crossterm::execute!(
                                stdout,
                                cursor::MoveTo(panel_col, row),
                                style::SetForegroundColor(self.theme.border),
                                style::SetBackgroundColor(self.theme.bg),
                                style::Print(border::VERTICAL),
                                style::SetForegroundColor(self.theme.description_fg),
                                style::Print(" "),
                                style::Print(line),
                                style::Print(" ".repeat(line_pad + 1)),
                                style::SetForegroundColor(self.theme.border),
                                style::Print(border::VERTICAL),
                            )?;
                        }

                        // Empty rows to fill panel height
                        let last_desc_row = popup_row + 3 + desc_lines.len() as u16;
                        let bottom_row_panel = popup_row + panel_height as u16 - 1;
                        for row in last_desc_row..bottom_row_panel {
                            crossterm::execute!(
                                stdout,
                                cursor::MoveTo(panel_col, row),
                                style::SetForegroundColor(self.theme.border),
                                style::SetBackgroundColor(self.theme.bg),
                                style::Print(border::VERTICAL),
                                style::Print(" ".repeat(panel_inner_width)),
                                style::Print(border::VERTICAL),
                            )?;
                        }

                        // Bottom border
                        crossterm::execute!(
                            stdout,
                            cursor::MoveTo(panel_col, bottom_row_panel),
                            style::SetForegroundColor(self.theme.border),
                            style::SetBackgroundColor(self.theme.bg),
                            style::Print(border::BOTTOM_LEFT),
                            style::Print(border::HORIZONTAL.repeat(panel_inner_width)),
                            style::Print(border::BOTTOM_RIGHT),
                        )?;
                    }
                }
            }
        }

        // Reset colors and restore cursor
        crossterm::execute!(
            stdout,
            style::ResetColor,
            cursor::RestorePosition,
        )?;

        stdout.flush()?;

        Ok((visible_count as u16 + 2, popup_col)) // (items + top/bottom border, actual col)
    }

    /// Clear the popup area.
    /// `cursor_row` and `popup_col` must be the values from the matching `render()` call
    /// (i.e. `popup_col` is the *actual* column `render()` used, not the raw cursor column).
    pub fn clear(
        &self,
        stdout: &mut impl Write,
        cursor_row: u16,
        popup_col: u16,
        lines: u16,
    ) -> std::io::Result<()> {
        if lines == 0 {
            return Ok(());
        }

        let (_, term_rows) = terminal::size().unwrap_or((80, 24));
        let popup_row = if cursor_row + 1 + lines > term_rows {
            cursor_row.saturating_sub(lines)
        } else {
            cursor_row + 1
        };

        crossterm::execute!(stdout, cursor::SavePosition)?;

        for i in 0..lines {
            crossterm::execute!(
                stdout,
                cursor::MoveTo(popup_col, popup_row + i),
                style::ResetColor,
                terminal::Clear(terminal::ClearType::UntilNewLine),
            )?;
        }

        crossterm::execute!(
            stdout,
            cursor::RestorePosition,
        )?;
        stdout.flush()?;
        Ok(())
    }
}
