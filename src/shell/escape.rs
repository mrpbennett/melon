use crate::input::parser::QuoteMode;

use super::detect::ShellType;

pub fn escape_fallback_completion(
    shell_type: &ShellType,
    quote_mode: QuoteMode,
    text: &str,
) -> String {
    match quote_mode {
        QuoteMode::None => escape_unquoted(text),
        QuoteMode::Single => escape_single_quoted(text),
        QuoteMode::Double => escape_double_quoted(shell_type, text),
    }
}

fn escape_unquoted(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if is_unquoted_safe(ch) {
            escaped.push(ch);
        } else {
            escaped.push('\\');
            escaped.push(ch);
        }
    }
    escaped
}

fn is_unquoted_safe(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':')
}

fn escape_single_quoted(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\'' {
            escaped.push_str("'\\''");
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

fn escape_double_quoted(shell_type: &ShellType, text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '"' | '$' | '`' | '\\')
            || (ch == '!' && !matches!(shell_type, ShellType::Fish))
        {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_unquoted_spaces_and_parens() {
        assert_eq!(
            escape_fallback_completion(&ShellType::Zsh, QuoteMode::None, "My Dir (1)/"),
            r"My\ Dir\ \(1\)/"
        );
    }

    #[test]
    fn test_escape_single_quoted_embedded_quote() {
        assert_eq!(
            escape_fallback_completion(&ShellType::Bash, QuoteMode::Single, "it's"),
            "it'\\''s"
        );
    }

    #[test]
    fn test_escape_double_quoted_shell_expansion_chars() {
        assert_eq!(
            escape_fallback_completion(&ShellType::Zsh, QuoteMode::Double, r#"$HOME"!"#),
            r#"\$HOME\"\!"#
        );
    }

    #[test]
    fn test_escape_double_quoted_fish_leaves_history_bang_alone() {
        assert_eq!(
            escape_fallback_completion(&ShellType::Fish, QuoteMode::Double, "wow!"),
            "wow!"
        );
    }
}
