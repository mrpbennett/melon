use crate::input::parser::QuoteMode;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LineState {
    buffer: String,
    cursor: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletionEdit {
    pub delete_left: usize,
    pub delete_right: usize,
    pub insert_text: String,
    pub move_left: usize,
    pub submits_line: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletionText {
    pub text: String,
    pub cursor: usize,
    pub submits_line: bool,
}

impl CompletionText {
    pub fn from_insert_value(value: &str) -> Self {
        let mut text = String::with_capacity(value.len());
        let mut cursor = None;
        let mut index = 0;

        while index < value.len() {
            if value[index..].starts_with("{cursor}") {
                cursor = Some(text.len());
                index += "{cursor}".len();
                continue;
            }

            let mut chars = value[index..].chars();
            let Some(ch) = chars.next() else {
                break;
            };
            let ch_len = ch.len_utf8();
            index += ch_len;

            if ch == '\u{8}' {
                if let Some((remove_at, _)) = text.char_indices().last() {
                    text.truncate(remove_at);
                    if let Some(position) = cursor.as_mut() {
                        *position = (*position).min(text.len());
                    }
                }
                continue;
            }

            text.push(ch);
        }

        let cursor = cursor.unwrap_or(text.len());
        let submits_line = text.contains('\n') || text.contains('\r');
        Self {
            text,
            cursor,
            submits_line,
        }
    }

    pub fn cursor_at_end(&self) -> bool {
        self.cursor == self.text.len()
    }
}

impl LineState {
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn before_cursor(&self) -> &str {
        &self.buffer[..self.cursor]
    }

    pub fn insert_bytes(&mut self, bytes: &[u8]) {
        if let Ok(text) = std::str::from_utf8(bytes) {
            self.insert_text(text);
        }
    }

    pub fn insert_text(&mut self, text: &str) {
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    pub fn backspace(&mut self) -> bool {
        let Some(prev) = self.prev_boundary(self.cursor) else {
            return false;
        };
        self.buffer.replace_range(prev..self.cursor, "");
        self.cursor = prev;
        true
    }

    pub fn delete(&mut self) -> bool {
        let Some(next) = self.next_boundary(self.cursor) else {
            return false;
        };
        self.buffer.replace_range(self.cursor..next, "");
        true
    }

    pub fn move_left(&mut self) -> bool {
        let Some(prev) = self.prev_boundary(self.cursor) else {
            return false;
        };
        self.cursor = prev;
        true
    }

    pub fn move_right(&mut self) -> bool {
        let Some(next) = self.next_boundary(self.cursor) else {
            return false;
        };
        self.cursor = next;
        true
    }

    pub fn move_home(&mut self) -> bool {
        let moved = self.cursor != 0;
        self.cursor = 0;
        moved
    }

    pub fn move_end(&mut self) -> bool {
        let moved = self.cursor != self.buffer.len();
        self.cursor = self.buffer.len();
        moved
    }

    pub fn kill_line(&mut self) -> usize {
        let removed = self.buffer[..self.cursor].chars().count();
        self.buffer.replace_range(..self.cursor, "");
        self.cursor = 0;
        removed
    }

    pub fn kill_last_word(&mut self) -> usize {
        let original_cursor = self.cursor;
        while let Some(prev) = self.prev_boundary(self.cursor) {
            let ch = self.buffer[prev..self.cursor].chars().next().unwrap_or(' ');
            if !ch.is_whitespace() {
                break;
            }
            self.cursor = prev;
        }

        while let Some(prev) = self.prev_boundary(self.cursor) {
            let ch = self.buffer[prev..self.cursor].chars().next().unwrap_or(' ');
            if ch.is_whitespace() {
                break;
            }
            self.cursor = prev;
        }

        let removed = self.buffer[self.cursor..original_cursor].chars().count();
        self.buffer.replace_range(self.cursor..original_cursor, "");
        removed
    }

    pub fn apply_completion(
        &mut self,
        replacement: &CompletionText,
        partial_chars: usize,
        append_space: bool,
    ) -> CompletionEdit {
        let start = self.byte_index_before_cursor(partial_chars);
        let token_end = if partial_chars == 0 {
            self.cursor
        } else {
            self.token_end_from_cursor()
        };
        self.apply_completion_span(replacement, start, token_end, append_space)
    }

    pub fn apply_completion_span(
        &mut self,
        replacement: &CompletionText,
        start: usize,
        end: usize,
        append_space: bool,
    ) -> CompletionEdit {
        let delete_left = self.buffer[start..self.cursor].chars().count();
        let delete_right = self.buffer[self.cursor..end].chars().count();

        let mut insert_text = replacement.text.clone();
        if append_space {
            insert_text.push(' ');
        }

        self.buffer.replace_range(start..end, &insert_text);
        let cursor = if append_space && replacement.cursor_at_end() {
            insert_text.len()
        } else {
            replacement.cursor
        };
        let move_left = insert_text[cursor..].chars().count();

        if replacement.submits_line {
            self.clear();
        } else {
            self.cursor = start + cursor;
        }

        CompletionEdit {
            delete_left,
            delete_right,
            insert_text,
            move_left,
            submits_line: replacement.submits_line,
        }
    }

    pub fn should_append_space(&self, insert_kind_is_folder: bool, partial_chars: usize) -> bool {
        self.should_append_space_for_span(
            insert_kind_is_folder,
            if partial_chars == 0 {
                self.cursor
            } else {
                self.token_end_from_cursor()
            },
            QuoteMode::None,
        )
    }

    pub fn should_append_space_for_span(
        &self,
        insert_kind_is_folder: bool,
        replacement_end: usize,
        quote_mode: QuoteMode,
    ) -> bool {
        if insert_kind_is_folder {
            return false;
        }

        if !matches!(quote_mode, QuoteMode::None) {
            return false;
        }

        self.cursor == replacement_end && replacement_end == self.buffer.len()
    }

    fn token_end_from_cursor(&self) -> usize {
        let mut index = self.cursor;
        while let Some(next) = self.next_boundary(index) {
            let ch = self.buffer[index..next].chars().next().unwrap_or(' ');
            if is_completion_boundary(ch) {
                break;
            }
            index = next;
        }
        index
    }

    fn byte_index_before_cursor(&self, chars: usize) -> usize {
        let mut index = self.cursor;
        for _ in 0..chars {
            let Some(prev) = self.prev_boundary(index) else {
                return 0;
            };
            index = prev;
        }
        index
    }

    fn prev_boundary(&self, at: usize) -> Option<usize> {
        if at == 0 {
            return None;
        }
        self.buffer[..at]
            .char_indices()
            .last()
            .map(|(index, _)| index)
    }

    fn next_boundary(&self, at: usize) -> Option<usize> {
        if at >= self.buffer.len() {
            return None;
        }
        self.buffer[at..]
            .char_indices()
            .nth(1)
            .map(|(offset, _)| at + offset)
            .or(Some(self.buffer.len()))
    }
}

fn is_completion_boundary(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '|' | '&' | ';' | '>' | '<')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::parser::{completion_edit_context, QuoteMode};

    #[test]
    fn test_insert_mid_line() {
        let mut line = LineState::default();
        line.insert_text("git commit");
        line.move_left();
        line.move_left();
        line.insert_text("x");
        assert_eq!(line.buffer(), "git commxit");
        assert_eq!(line.cursor(), "git commx".len());
    }

    #[test]
    fn test_apply_completion_replaces_suffix_of_current_token() {
        let mut line = LineState::default();
        line.insert_text("git cheout");
        line.move_left();
        line.move_left();
        line.move_left();
        let edit = line.apply_completion(&CompletionText::from_insert_value("checkout"), 3, true);
        assert_eq!(
            edit,
            CompletionEdit {
                delete_left: 3,
                delete_right: 3,
                insert_text: "checkout ".into(),
                move_left: 0,
                submits_line: false,
            }
        );
        assert_eq!(line.buffer(), "git checkout ");
    }

    #[test]
    fn test_completion_text_respects_cursor_marker() {
        let completion = CompletionText::from_insert_value("-m '{cursor}'");
        assert_eq!(completion.text, "-m ''");
        assert_eq!(completion.cursor, 4);
        assert!(!completion.submits_line);
    }

    #[test]
    fn test_completion_text_applies_backspace_escape() {
        let completion = CompletionText::from_insert_value("foo\u{8}bar");
        assert_eq!(completion.text, "fobar");
        assert_eq!(completion.cursor, "fobar".len());
    }

    #[test]
    fn test_apply_completion_moves_cursor_left_for_internal_cursor_marker() {
        let mut line = LineState::default();
        line.insert_text("git commit ");
        let completion = CompletionText::from_insert_value("-m '{cursor}'");
        let edit = line.apply_completion(&completion, 0, false);

        assert_eq!(
            edit,
            CompletionEdit {
                delete_left: 0,
                delete_right: 0,
                insert_text: "-m ''".into(),
                move_left: 1,
                submits_line: false,
            }
        );
        assert_eq!(line.buffer(), "git commit -m ''");
        assert_eq!(line.cursor(), "git commit -m '".len());
    }

    #[test]
    fn test_apply_completion_span_replaces_raw_escaped_token() {
        let mut line = LineState::default();
        line.insert_text(r"echo hello\ world");
        let context = completion_edit_context(line.buffer(), line.cursor());
        let completion = CompletionText::from_insert_value(r"good\ bye");
        let edit = line.apply_completion_span(
            &completion,
            context.replacement_start,
            context.replacement_end,
            false,
        );

        assert_eq!(
            edit,
            CompletionEdit {
                delete_left: 12,
                delete_right: 0,
                insert_text: r"good\ bye".into(),
                move_left: 0,
                submits_line: false,
            }
        );
        assert_eq!(line.buffer(), r"echo good\ bye");
    }

    #[test]
    fn test_kill_last_word_respects_cursor() {
        let mut line = LineState::default();
        line.insert_text("git commit --amend");
        line.move_left();
        line.move_left();
        let removed = line.kill_last_word();
        assert_eq!(removed, 5);
        assert_eq!(line.buffer(), "git commit nd");
    }

    #[test]
    fn test_should_append_space_only_at_end_of_token() {
        let mut line = LineState::default();
        line.insert_text("git cheout");
        line.move_left();
        line.move_left();
        line.move_left();
        assert!(!line.should_append_space(false, 3));
        line.move_end();
        assert!(line.should_append_space(false, 3));
        assert!(!line.should_append_space(true, 3));
    }

    #[test]
    fn test_should_append_space_for_new_token_at_boundary() {
        let mut line = LineState::default();
        line.insert_text("git ");
        assert!(line.should_append_space(false, 0));

        line.insert_text("commit");
        line.move_home();
        assert!(!line.should_append_space(false, 0));
    }

    #[test]
    fn test_should_not_append_space_inside_quotes() {
        let mut line = LineState::default();
        line.insert_text("echo \"hello");
        let context = completion_edit_context(line.buffer(), line.cursor());
        assert!(!line.should_append_space_for_span(
            false,
            context.replacement_end,
            QuoteMode::Double
        ));
    }
}
