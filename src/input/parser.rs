/// Command-line tokenizer that handles quotes, escapes, pipes, and operators.

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

/// Parse a command line into tokens. Returns the tokens for the *last* simple
/// command (after any pipe/operator), which is the one we want to complete.
pub fn tokenize_last_command(input: &str) -> Vec<Token> {
    let all = tokenize(input);
    // Find the last pipe/operator and return everything after it
    let mut last_cmd_start = 0;
    for (i, tok) in all.iter().enumerate() {
        if matches!(tok.kind, TokenKind::Pipe | TokenKind::Operator) {
            last_cmd_start = i + 1;
        }
    }
    all[last_cmd_start..].to_vec()
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
            tokens.push(Token { text: "&&".into(), kind: TokenKind::Operator, start: i });
            i += 2;
            continue;
        }
        if i + 1 < len && bytes[i] == b'|' && bytes[i + 1] == b'|' {
            tokens.push(Token { text: "||".into(), kind: TokenKind::Operator, start: i });
            i += 2;
            continue;
        }
        if bytes[i] == b'|' {
            tokens.push(Token { text: "|".into(), kind: TokenKind::Pipe, start: i });
            i += 1;
            continue;
        }
        if bytes[i] == b';' {
            tokens.push(Token { text: ";".into(), kind: TokenKind::Operator, start: i });
            i += 1;
            continue;
        }

        // Redirects
        if bytes[i] == b'>' || bytes[i] == b'<' || (bytes[i] == b'2' && i + 1 < len && bytes[i + 1] == b'>') {
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
            tokens.push(Token { text, kind: TokenKind::Redirect, start });
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
                    if i < len { i += 1; } // skip closing quote
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
                    if i < len { i += 1; } // skip closing quote
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
            tokens.push(Token { text: word, kind: TokenKind::Word, start });
        }
    }

    tokens
}

/// Determine if the cursor is in a "partial word" at the end of input.
/// Returns (tokens_before_partial, partial_text).
pub fn split_partial(input: &str) -> (Vec<Token>, String) {
    let tokens = tokenize_last_command(input);
    if input.ends_with(' ') || input.is_empty() {
        // Cursor is after a space — no partial word
        (tokens, String::new())
    } else if let Some(last) = tokens.last() {
        let partial = last.text.clone();
        let rest = tokens[..tokens.len() - 1].to_vec();
        (rest, partial)
    } else {
        (vec![], String::new())
    }
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
}
