use crate::completion::matcher::ScoredCandidate;

/// State for the autocomplete popup.
pub struct PopupState {
    /// All candidates (pre-filtered and scored).
    pub items: Vec<ScoredCandidate>,
    /// Currently selected index.
    pub selected: usize,
    /// Scroll offset (first visible item index).
    pub scroll_offset: usize,
    /// Maximum number of visible items.
    pub max_visible: usize,
    /// Whether the popup is active/visible.
    pub visible: bool,
}

impl PopupState {
    pub fn new(max_visible: usize) -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            max_visible,
            visible: false,
        }
    }

    /// Update the popup with new candidates. Resets selection.
    pub fn set_items(&mut self, items: Vec<ScoredCandidate>) {
        self.visible = !items.is_empty();
        self.items = items;
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Update the popup while preserving selection when the same candidate still exists.
    pub fn set_items_preserve_selection(&mut self, items: Vec<ScoredCandidate>) {
        let selected_name = self.selected_item().map(|item| item.candidate.name.clone());
        self.visible = !items.is_empty();
        self.items = items;

        if let Some(selected_name) = selected_name {
            if let Some(index) = self
                .items
                .iter()
                .position(|item| item.candidate.name == selected_name)
            {
                self.selected = index;
                self.ensure_visible();
                return;
            }
        }

        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.items.len();
        self.ensure_visible();
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        if self.items.is_empty() {
            return;
        }
        if self.selected == 0 {
            self.selected = self.items.len() - 1;
        } else {
            self.selected -= 1;
        }
        self.ensure_visible();
    }

    /// Page down.
    pub fn page_down(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.selected = (self.selected + self.max_visible).min(self.items.len() - 1);
        self.ensure_visible();
    }

    /// Page up.
    pub fn page_up(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(self.max_visible);
        self.ensure_visible();
    }

    /// Get the currently selected item's completion text.
    pub fn selected_text(&self) -> Option<&str> {
        self.items
            .get(self.selected)
            .map(|s| s.candidate.name.as_str())
    }

    pub fn selected_item(&self) -> Option<&ScoredCandidate> {
        self.items.get(self.selected)
    }

    /// Dismiss the popup.
    pub fn dismiss(&mut self) {
        self.visible = false;
        self.items.clear();
        self.selected = 0;
        self.scroll_offset = 0;
    }

    /// Number of visible items (clamped to max_visible).
    pub fn visible_count(&self) -> usize {
        self.items.len().min(self.max_visible)
    }

    /// Ensure the selected item is within the visible scroll window.
    fn ensure_visible(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + self.max_visible {
            self.scroll_offset = self.selected - self.max_visible + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::spec::{CandidateKind, CompletionCandidate};

    fn scored(name: &str) -> ScoredCandidate {
        ScoredCandidate {
            candidate: CompletionCandidate {
                name: name.to_string(),
                insert_value: None,
                display_name: None,
                description: None,
                icon: None,
                priority: 50,
                kind: CandidateKind::Subcommand,
            },
            score: 100,
        }
    }

    #[test]
    fn test_selection_wraps() {
        let mut state = PopupState::new(5);
        state.set_items(vec![scored("a"), scored("b"), scored("c")]);
        assert_eq!(state.selected, 0);
        state.select_next();
        assert_eq!(state.selected, 1);
        state.select_next();
        assert_eq!(state.selected, 2);
        state.select_next();
        assert_eq!(state.selected, 0); // wraps
    }

    #[test]
    fn test_selection_wraps_up() {
        let mut state = PopupState::new(5);
        state.set_items(vec![scored("a"), scored("b"), scored("c")]);
        state.select_prev();
        assert_eq!(state.selected, 2); // wraps to end
    }

    #[test]
    fn test_scroll() {
        let mut state = PopupState::new(2);
        state.set_items(vec![scored("a"), scored("b"), scored("c"), scored("d")]);
        assert_eq!(state.scroll_offset, 0);
        state.select_next(); // 1
        state.select_next(); // 2 → scroll
        assert_eq!(state.scroll_offset, 1);
    }

    #[test]
    fn test_dismiss() {
        let mut state = PopupState::new(5);
        state.set_items(vec![scored("a")]);
        assert!(state.visible);
        state.dismiss();
        assert!(!state.visible);
        assert!(state.items.is_empty());
    }

    #[test]
    fn test_selection_preserved_when_candidate_still_exists() {
        let mut state = PopupState::new(5);
        state.set_items(vec![scored("alpha"), scored("beta"), scored("gamma")]);
        state.select_next();
        state.set_items_preserve_selection(vec![scored("beta"), scored("delta")]);
        assert_eq!(state.selected_text(), Some("beta"));
    }
}
