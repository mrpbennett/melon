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
        replacement: &str,
        partial_chars: usize,
        append_space: bool,
    ) -> CompletionEdit {
        let start = self.byte_index_before_cursor(partial_chars);
        let delete_left = self.buffer[start..self.cursor].chars().count();
        let token_end = if partial_chars == 0 {
            self.cursor
        } else {
            self.token_end_from_cursor()
        };
        let delete_right = self.buffer[self.cursor..token_end].chars().count();

        let mut insert_text = replacement.to_string();
        if append_space {
            insert_text.push(' ');
        }

        self.buffer.replace_range(start..token_end, &insert_text);
        self.cursor = start + insert_text.len();

        CompletionEdit {
            delete_left,
            delete_right,
            insert_text,
        }
    }

    pub fn should_append_space(&self, insert_kind_is_folder: bool, partial_chars: usize) -> bool {
        if insert_kind_is_folder {
            return false;
        }

        if self.cursor < self.buffer.len() {
            return false;
        }

        if partial_chars == 0 {
            return true;
        }

        self.token_end_from_cursor() == self.cursor
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
        let edit = line.apply_completion("checkout", 3, true);
        assert_eq!(
            edit,
            CompletionEdit {
                delete_left: 3,
                delete_right: 3,
                insert_text: "checkout ".into(),
            }
        );
        assert_eq!(line.buffer(), "git checkout ");
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
}
