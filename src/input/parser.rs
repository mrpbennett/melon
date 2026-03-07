/// Command-line tokenizer that handles quotes, escapes, pipes, and operators.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteMode {
    None,
    Single,
    Double,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionEditContext {
    pub replacement_start: usize,
    pub replacement_end: usize,
    pub quote_mode: QuoteMode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub text: String,
    pub kind: TokenKind,
    /// Byte offset in the original input where this token starts.
    pub start: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// A regular word/argument.
    Word,
    /// A pipe `|`.
    Pipe,
    /// `&&` or `||` or `;`.
    Operator,
    /// A redirect like `>`, `>>`, `<`, `2>`.
    Redirect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLine {
    pub tokens: Vec<Token>,
    pub partial: String,
}

/// Parse a command line into tokens. Returns the tokens for the *last* simple
/// command (after any pipe/operator), which is the one we want to complete.
pub fn tokenize_last_command(input: &str) -> Vec<Token> {
    let mut all = tokenize(input);
    // Find the last pipe/operator and return everything after it
    let mut last_cmd_start = 0;
    for (i, tok) in all.iter().enumerate() {
        if matches!(tok.kind, TokenKind::Pipe | TokenKind::Operator) {
            last_cmd_start = i + 1;
        }
    }
    all.split_off(last_cmd_start)
}

/// Full tokenizer.
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Check for operators
        if i + 1 < len && bytes[i] == b'&' && bytes[i + 1] == b'&' {
            tokens.push(Token {
                text: "&&".into(),
                kind: TokenKind::Operator,
                start: i,
            });
            i += 2;
            continue;
        }
        if i + 1 < len && bytes[i] == b'|' && bytes[i + 1] == b'|' {
            tokens.push(Token {
                text: "||".into(),
                kind: TokenKind::Operator,
                start: i,
            });
            i += 2;
            continue;
        }
        if bytes[i] == b'|' {
            tokens.push(Token {
                text: "|".into(),
                kind: TokenKind::Pipe,
                start: i,
            });
            i += 1;
            continue;
        }
        if bytes[i] == b';' {
            tokens.push(Token {
                text: ";".into(),
                kind: TokenKind::Operator,
                start: i,
            });
            i += 1;
            continue;
        }

        // Redirects
        if bytes[i] == b'>'
            || bytes[i] == b'<'
            || (bytes[i] == b'2' && i + 1 < len && bytes[i + 1] == b'>')
        {
            let start = i;
            let mut text = String::new();
            if bytes[i] == b'2' {
                text.push('2');
                i += 1;
            }
            text.push(bytes[i] as char);
            i += 1;
            if i < len && bytes[i] == b'>' {
                text.push('>');
                i += 1;
            }
            tokens.push(Token {
                text,
                kind: TokenKind::Redirect,
                start,
            });
            continue;
        }

        // Word (possibly quoted)
        let start = i;
        let mut word = String::new();
        while i < len && !bytes[i].is_ascii_whitespace() {
            match bytes[i] {
                b'\'' => {
                    // Single-quoted string: everything until closing quote
                    i += 1;
                    while i < len && bytes[i] != b'\'' {
                        word.push(bytes[i] as char);
                        i += 1;
                    }
                    if i < len {
                        i += 1;
                    } // skip closing quote
                }
                b'"' => {
                    // Double-quoted string: allows backslash escapes
                    i += 1;
                    while i < len && bytes[i] != b'"' {
                        if bytes[i] == b'\\' && i + 1 < len {
                            i += 1;
                            word.push(bytes[i] as char);
                        } else {
                            word.push(bytes[i] as char);
                        }
                        i += 1;
                    }
                    if i < len {
                        i += 1;
                    } // skip closing quote
                }
                b'\\' if i + 1 < len => {
                    i += 1;
                    word.push(bytes[i] as char);
                    i += 1;
                }
                // Stop at operators
                b'|' | b'&' | b';' | b'>' | b'<' => break,
                c => {
                    word.push(c as char);
                    i += 1;
                }
            }
        }
        if !word.is_empty() {
            tokens.push(Token {
                text: word,
                kind: TokenKind::Word,
                start,
            });
        }
    }

    tokens
}

/// Determine if the cursor is in a "partial word" at the end of input.
/// Returns (tokens_before_partial, partial_text).
pub fn parse_completion_input(input: &str) -> ParsedLine {
    let mut tokens = tokenize_last_command(input);
    if input.ends_with(' ') || input.is_empty() {
        ParsedLine {
            tokens,
            partial: String::new(),
        }
    } else {
        let partial = tokens.pop().map(|token| token.text).unwrap_or_default();
        ParsedLine { tokens, partial }
    }
}

pub fn split_partial(input: &str) -> (Vec<Token>, String) {
    let parsed = parse_completion_input(input);
    (parsed.tokens, parsed.partial)
}

pub fn completion_edit_context(input: &str, cursor: usize) -> CompletionEditContext {
    let scan = scan_to_cursor(input, cursor);
    let replacement_start = if scan.preserve_open_quote() {
        scan.quote_start
            .map(|index| index + 1)
            .unwrap_or(scan.token_start)
    } else {
        scan.token_start
    };
    let replacement_end = scan_forward_for_replacement_end(input, cursor, &scan);

    CompletionEditContext {
        replacement_start,
        replacement_end,
        quote_mode: scan.quote_mode,
    }
}

#[derive(Debug, Clone, Copy)]
struct CursorScan {
    token_start: usize,
    quote_mode: QuoteMode,
    quote_start: Option<usize>,
    quote_started_at_token_start: bool,
}

impl CursorScan {
    fn preserve_open_quote(&self) -> bool {
        !matches!(self.quote_mode, QuoteMode::None) && self.quote_started_at_token_start
    }
}

fn scan_to_cursor(input: &str, cursor: usize) -> CursorScan {
    let mut token_start = cursor;
    let mut in_token = false;
    let mut quote_mode = QuoteMode::None;
    let mut quote_start = None;
    let mut quote_started_at_token_start = false;
    let mut escaped = false;

    for (index, ch) in input.char_indices() {
        if index >= cursor {
            break;
        }

        match quote_mode {
            QuoteMode::None => {
                if escaped {
                    escaped = false;
                    continue;
                }

                if is_completion_boundary(ch) {
                    in_token = false;
                    token_start = index + ch.len_utf8();
                    continue;
                }

                if !in_token {
                    in_token = true;
                    token_start = index;
                }

                match ch {
                    '\\' => escaped = true,
                    '\'' => {
                        quote_mode = QuoteMode::Single;
                        quote_start = Some(index);
                        quote_started_at_token_start = index == token_start;
                    }
                    '"' => {
                        quote_mode = QuoteMode::Double;
                        quote_start = Some(index);
                        quote_started_at_token_start = index == token_start;
                    }
                    _ => {}
                }
            }
            QuoteMode::Single => {
                if ch == '\'' {
                    quote_mode = QuoteMode::None;
                    quote_start = None;
                }
            }
            QuoteMode::Double => {
                if escaped {
                    escaped = false;
                    continue;
                }

                match ch {
                    '\\' => escaped = true,
                    '"' => {
                        quote_mode = QuoteMode::None;
                        quote_start = None;
                    }
                    _ => {}
                }
            }
        }
    }

    if !in_token {
        token_start = cursor;
    }

    CursorScan {
        token_start,
        quote_mode,
        quote_start,
        quote_started_at_token_start,
    }
}

fn scan_forward_for_replacement_end(input: &str, cursor: usize, scan: &CursorScan) -> usize {
    let mut quote_mode = scan.quote_mode;
    let mut escaped = false;
    let mut end = cursor;

    for (offset, ch) in input[cursor..].char_indices() {
        let index = cursor + offset;

        match quote_mode {
            QuoteMode::None => {
                if escaped {
                    escaped = false;
                    end = index + ch.len_utf8();
                    continue;
                }

                if is_completion_boundary(ch) {
                    return index;
                }

                match ch {
                    '\\' => escaped = true,
                    '\'' => quote_mode = QuoteMode::Single,
                    '"' => quote_mode = QuoteMode::Double,
                    _ => {}
                }
                end = index + ch.len_utf8();
            }
            QuoteMode::Single => {
                if ch == '\'' {
                    if scan.preserve_open_quote() {
                        return index;
                    }
                    quote_mode = QuoteMode::None;
                }
                end = index + ch.len_utf8();
            }
            QuoteMode::Double => {
                if escaped {
                    escaped = false;
                    end = index + ch.len_utf8();
                    continue;
                }

                match ch {
                    '\\' => escaped = true,
                    '"' => {
                        if scan.preserve_open_quote() {
                            return index;
                        }
                        quote_mode = QuoteMode::None;
                    }
                    _ => {}
                }
                end = index + ch.len_utf8();
            }
        }
    }

    if scan.preserve_open_quote() {
        cursor
    } else {
        end
    }
}

fn is_completion_boundary(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '|' | '&' | ';' | '>' | '<')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_tokens() {
        let tokens = tokenize("git commit -m 'hello world'");
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[0].text, "git");
        assert_eq!(tokens[1].text, "commit");
        assert_eq!(tokens[2].text, "-m");
        assert_eq!(tokens[3].text, "hello world");
    }

    #[test]
    fn test_pipes() {
        let tokens = tokenize("cat file | grep pattern");
        assert_eq!(tokens.len(), 5);
        assert_eq!(tokens[2].kind, TokenKind::Pipe);

        let last = tokenize_last_command("cat file | grep pattern");
        assert_eq!(last.len(), 2);
        assert_eq!(last[0].text, "grep");
    }

    #[test]
    fn test_operators() {
        let last = tokenize_last_command("make && cargo test");
        assert_eq!(last.len(), 2);
        assert_eq!(last[0].text, "cargo");
    }

    #[test]
    fn test_split_partial() {
        let (tokens, partial) = split_partial("git com");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].text, "git");
        assert_eq!(partial, "com");
    }

    #[test]
    fn test_split_partial_trailing_space() {
        let (tokens, partial) = split_partial("git ");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].text, "git");
        assert_eq!(partial, "");
    }

    #[test]
    fn test_parse_completion_input_reuses_last_token_as_partial() {
        let parsed = parse_completion_input("git commit");
        assert_eq!(parsed.tokens.len(), 1);
        assert_eq!(parsed.tokens[0].text, "git");
        assert_eq!(parsed.partial, "commit");
    }

    #[test]
    fn test_double_quotes_with_escape() {
        let tokens = tokenize(r#"echo "hello \"world""#);
        assert_eq!(tokens[1].text, r#"hello "world"#);
    }

    #[test]
    fn test_backslash_escape() {
        let tokens = tokenize(r"echo hello\ world");
        assert_eq!(tokens[1].text, "hello world");
    }

    #[test]
    fn test_redirects() {
        let tokens = tokenize("echo hello > file.txt");
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[2].kind, TokenKind::Redirect);
    }

    #[test]
    fn test_completion_edit_context_replaces_full_escaped_token() {
        let input = r"echo hello\ world";
        let context = completion_edit_context(input, input.len());
        assert_eq!(
            context,
            CompletionEditContext {
                replacement_start: 5,
                replacement_end: input.len(),
                quote_mode: QuoteMode::None,
            }
        );
    }

    #[test]
    fn test_completion_edit_context_preserves_opening_quote() {
        let input = "echo \"hello world";
        let context = completion_edit_context(input, input.len());
        assert_eq!(
            context,
            CompletionEditContext {
                replacement_start: 6,
                replacement_end: input.len(),
                quote_mode: QuoteMode::Double,
            }
        );
    }

    #[test]
    fn test_completion_edit_context_preserves_closing_quote_after_cursor() {
        let input = "echo \"hello\"";
        let cursor = input.len() - 1;
        let context = completion_edit_context(input, cursor);
        assert_eq!(
            context,
            CompletionEditContext {
                replacement_start: 6,
                replacement_end: cursor,
                quote_mode: QuoteMode::Double,
            }
        );
    }
}
