use crossterm::cursor;
use crossterm::style;
use crossterm::terminal;
use std::borrow::Cow;
use std::io::Write;
use unicode_width::UnicodeWidthStr;

use super::popup::PopupState;
use super::theme::{border, Theme};
use crate::completion::spec::{CandidateKind, CompletionCandidate};

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

fn truncate_to_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let mut out = String::new();
    for ch in text.chars() {
        out.push(ch);
        if UnicodeWidthStr::width(out.as_str()) > max_width {
            out.pop();
            break;
        }
    }
    out
}

fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }

    let mut truncated = truncate_to_width(text, max_width - 1);
    truncated.push('…');
    truncated
}

fn fallback_icon(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Subcommand => "❯",
        CandidateKind::Option => "⌥",
        CandidateKind::Argument => "•",
        CandidateKind::File => "📄",
        CandidateKind::Folder => "📁",
    }
}

fn fig_icon_fallback(icon: &str, kind: CandidateKind) -> &'static str {
    let icon_type = icon
        .split('?')
        .nth(1)
        .and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("type=")));

    match icon_type {
        Some("folder" | "dir" | "directory") => "📁",
        Some("file" | "text" | "document") => "📄",
        Some("git" | "branch") => "🌿",
        Some("node" | "commit") => "●",
        Some("commandkey") => "⌘",
        Some("asterisk") => "✱",
        Some("box" | "package" | "pkg") => "📦",
        Some("docker") => "🐳",
        Some("warning" | "alert") => "⚠",
        Some("link" | "url") => "🔗",
        Some("cloud") => "☁",
        _ => fallback_icon(kind),
    }
}

fn terminal_icon(candidate: &CompletionCandidate) -> Cow<'_, str> {
    match candidate.icon.as_deref() {
        Some(icon) if icon.trim().is_empty() => {
            Cow::Borrowed(fallback_icon(candidate.kind.clone()))
        }
        Some(icon) if icon.starts_with("fig://") => {
            Cow::Borrowed(fig_icon_fallback(icon, candidate.kind.clone()))
        }
        Some(icon) if icon.starts_with("http://") || icon.starts_with("https://") => {
            Cow::Borrowed("🌐")
        }
        Some(icon) => Cow::Borrowed(icon),
        None => Cow::Borrowed(fallback_icon(candidate.kind.clone())),
    }
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
        let mut max_icon_width = 0usize;
        let mut max_name_width = 0usize;
        let mut max_desc_width = 0usize;
        for i in state.scroll_offset..state.scroll_offset + visible_count {
            if let Some(item) = state.items.get(i) {
                let icon = terminal_icon(&item.candidate);
                max_icon_width = max_icon_width.max(UnicodeWidthStr::width(icon.as_ref()));
                let name_w = UnicodeWidthStr::width(item.candidate.display_label());
                max_name_width = max_name_width.max(name_w);
                if let Some(desc) = &item.candidate.description {
                    max_desc_width = max_desc_width.max(UnicodeWidthStr::width(desc.as_str()));
                }
            }
        }

        let icon_col_width = max_icon_width.max(1);
        let desc_target_width = max_desc_width.min(30);
        let has_descriptions = max_desc_width > 0;
        let content_width = if has_descriptions {
            icon_col_width + 1 + max_name_width + 4 + desc_target_width
        } else {
            icon_col_width + 1 + max_name_width
        };
        let inner_width = content_width.clamp(self.theme.min_width, self.theme.max_width);
        let popup_width = inner_width + 2; // borders
        let available_after_icon = inner_width.saturating_sub(icon_col_width + 1);
        let reserved_desc_width = if has_descriptions {
            desc_target_width.min(available_after_icon.saturating_sub(8))
        } else {
            0
        };
        let name_col_width = if has_descriptions {
            available_after_icon.saturating_sub(4 + reserved_desc_width)
        } else {
            available_after_icon
        }
        .max(1);
        let popup_height = visible_count as u16 + 2;

        let panel_height = if self.theme.show_description_panel {
            state
                .items
                .get(state.selected)
                .and_then(|selected| selected.candidate.description.as_ref())
                .map(|desc| {
                    let panel_inner_width: usize = 36;
                    let wrap_width = panel_inner_width.saturating_sub(2);
                    let desc_lines = word_wrap(desc, wrap_width);
                    let content_lines = 1 + desc_lines.len();
                    (visible_count + 2).max(content_lines + 3) as u16
                })
                .unwrap_or(popup_height)
        } else {
            popup_height
        };

        let popup_col = if cursor_col + popup_width as u16 > term_cols {
            term_cols.saturating_sub(popup_width as u16)
        } else {
            cursor_col
        };
        let panel_col = popup_col + popup_width as u16 + 1;
        let panel_inner_width: usize = 36;
        let panel_width = panel_inner_width + 2;
        let renders_panel = self.theme.show_description_panel
            && state
                .items
                .get(state.selected)
                .and_then(|selected| selected.candidate.description.as_ref())
                .is_some()
            && panel_col + panel_width as u16 <= term_cols;
        let rendered_height = if renders_panel {
            popup_height.max(panel_height)
        } else {
            popup_height
        };

        // Position: below cursor, clamped to terminal bounds
        let popup_row = if cursor_row + 1 + rendered_height > term_rows {
            // Not enough space below — draw above
            cursor_row.saturating_sub(rendered_height)
        } else {
            cursor_row + 1
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
            let bg = if is_selected {
                self.theme.selected_bg
            } else {
                self.theme.bg
            };
            let fg = if is_selected {
                self.theme.selected_fg
            } else {
                self.theme.fg
            };

            let icon = terminal_icon(&item.candidate);
            let icon_width = UnicodeWidthStr::width(icon.as_ref());
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

            let name = item.candidate.display_label();
            let name_text = truncate_with_ellipsis(name, name_col_width);
            let name_width = UnicodeWidthStr::width(name_text.as_str());
            let name_padding = name_col_width.saturating_sub(name_width);

            let row = popup_row + 1 + i as u16;
            crossterm::execute!(
                stdout,
                cursor::MoveTo(popup_col, row),
                style::SetForegroundColor(left_border_fg),
                style::SetBackgroundColor(bg),
                style::Print(left_border),
                style::SetForegroundColor(icon_fg),
                style::Print(icon),
                style::Print(" ".repeat(icon_col_width.saturating_sub(icon_width))),
                style::Print(" "),
                style::SetForegroundColor(fg),
                style::Print(&name_text),
                style::Print(" ".repeat(name_padding)),
            )?;

            // Description (if fits)
            let remaining = inner_width.saturating_sub(icon_col_width + 1 + name_col_width);
            if has_descriptions && remaining > 5 {
                let desc = item.candidate.description.as_deref().unwrap_or("");
                let desc_max = remaining.saturating_sub(4);
                let truncated = truncate_with_ellipsis(desc, desc_max);
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
        if renders_panel {
            if let Some(selected) = state.items.get(state.selected) {
                if let Some(desc) = &selected.candidate.description {
                    let name = selected.candidate.display_label();
                    // Word-wrap description into lines of panel_inner_width
                    let wrap_width = panel_inner_width.saturating_sub(2); // 1-char padding each side
                    let desc_lines = word_wrap(desc, wrap_width);

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
                    let panel_name = truncate_with_ellipsis(name, wrap_width);
                    let name_w = UnicodeWidthStr::width(panel_name.as_str());
                    let name_pad = wrap_width.saturating_sub(name_w);
                    crossterm::execute!(
                        stdout,
                        cursor::MoveTo(panel_col, popup_row + 1),
                        style::SetForegroundColor(self.theme.border),
                        style::SetBackgroundColor(self.theme.bg),
                        style::Print(border::VERTICAL),
                        style::SetForegroundColor(self.theme.selected_fg),
                        style::Print(" "),
                        style::Print(&panel_name),
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
                        if row >= popup_row + rendered_height - 1 {
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
                    let bottom_row_panel = popup_row + rendered_height - 1;
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

        // Reset colors and restore cursor
        crossterm::execute!(stdout, style::ResetColor, cursor::RestorePosition,)?;

        stdout.flush()?;

        Ok((rendered_height, popup_col))
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

        crossterm::execute!(stdout, cursor::RestorePosition,)?;
        stdout.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::matcher::ScoredCandidate;

    fn candidate(
        name: &str,
        display_name: Option<&str>,
        icon: Option<&str>,
    ) -> CompletionCandidate {
        CompletionCandidate {
            name: name.to_string(),
            insert_value: None,
            display_name: display_name.map(str::to_string),
            description: Some("Description".into()),
            icon: icon.map(str::to_string),
            priority: 50,
            kind: CandidateKind::Subcommand,
        }
    }

    #[test]
    fn test_terminal_icon_uses_emoji_icon_verbatim() {
        let candidate = candidate("git", Some("Git 🌿"), Some("🌿"));
        assert_eq!(terminal_icon(&candidate), Cow::Borrowed("🌿"));
    }

    #[test]
    fn test_terminal_icon_maps_fig_protocol_icons() {
        let candidate = candidate("git", None, Some("fig://icon?type=git"));
        assert_eq!(terminal_icon(&candidate), Cow::Borrowed("🌿"));
    }

    #[test]
    fn test_render_uses_display_name_and_icon() {
        let renderer = PopupRenderer::new(Theme::default());
        let mut popup = PopupState::new(5);
        popup.set_items(vec![ScoredCandidate {
            candidate: candidate("commit", Some("Commit ✍️"), Some("🌿")),
            score: 10,
        }]);

        let mut output = Vec::new();
        renderer.render(&mut output, &popup, 0, 0).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("🌿"));
        assert!(rendered.contains("Commit ✍️"));
    }

    #[test]
    fn test_render_reports_panel_height_when_description_panel_is_taller() {
        let mut theme = Theme::default();
        theme.show_description_panel = true;
        theme.max_width = 30;
        let renderer = PopupRenderer::new(theme);
        let mut popup = PopupState::new(5);
        popup.set_items(vec![ScoredCandidate {
            candidate: CompletionCandidate {
                name: "commit".to_string(),
                insert_value: None,
                display_name: Some("Commit".into()),
                description: Some(
                    "This description is intentionally long so the side panel wraps across multiple lines and exceeds the main popup height.".into(),
                ),
                icon: Some("🌿".into()),
                priority: 50,
                kind: CandidateKind::Subcommand,
            },
            score: 10,
        }]);

        let mut output = Vec::new();
        let (lines, _) = renderer.render(&mut output, &popup, 0, 0).unwrap();
        assert!(lines > popup.visible_count() as u16 + 2);
    }

    #[test]
    fn test_truncate_with_ellipsis_preserves_unicode_boundaries() {
        let truncated = truncate_with_ellipsis("Commit ✍️ with extras", 10);
        assert!(UnicodeWidthStr::width(truncated.as_str()) <= 10);
        assert!(truncated.ends_with('…'));
    }
}
