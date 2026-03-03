use crossterm::style::Color;

/// Visual theme for the autocomplete popup.
pub struct Theme {
    /// Border color.
    pub border: Color,
    /// Background color for the popup.
    pub bg: Color,
    /// Text color for normal items.
    pub fg: Color,
    /// Background for the selected item.
    pub selected_bg: Color,
    /// Text color for the selected item.
    pub selected_fg: Color,
    /// Color for descriptions.
    pub description_fg: Color,
    /// Color for the matched characters in fuzzy matching.
    pub match_fg: Color,
    /// Maximum number of visible items.
    pub max_visible: usize,
    /// Minimum popup width.
    pub min_width: usize,
    /// Maximum popup width.
    pub max_width: usize,
    /// Show a description panel to the right of the popup for the selected item.
    pub show_description_panel: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            border: Color::Rgb { r: 88, g: 91, b: 112 },
            bg: Color::Rgb { r: 30, g: 30, b: 46 },
            fg: Color::Rgb { r: 205, g: 214, b: 244 },
            selected_bg: Color::Rgb { r: 49, g: 50, b: 68 },
            selected_fg: Color::Rgb { r: 205, g: 214, b: 244 },
            description_fg: Color::Rgb { r: 108, g: 112, b: 134 },
            match_fg: Color::Rgb { r: 137, g: 180, b: 250 },
max_visible: 8,
            min_width: 20,
            max_width: 60,
            show_description_panel: false,
        }
    }
}

/// Box-drawing characters for rounded borders.
pub mod border {
    pub const TOP_LEFT: &str = "╭";
    pub const TOP_RIGHT: &str = "╮";
    pub const BOTTOM_LEFT: &str = "╰";
    pub const BOTTOM_RIGHT: &str = "╯";
    pub const HORIZONTAL: &str = "─";
    pub const VERTICAL: &str = "│";
}
