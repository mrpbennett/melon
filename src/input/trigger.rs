/// Key classification for the completion state machine.

/// Actions that the input processor can produce.
#[derive(Debug, Clone, PartialEq)]
pub enum InputAction {
    /// Normal character — passthrough to PTY.
    Passthrough(Vec<u8>),
    /// Tab pressed — activate/cycle completion.
    Tab,
    /// Shift+Tab — cycle completion backwards.
    ShiftTab,
    /// Arrow down — select next item in popup.
    Down,
    /// Arrow up — select previous item in popup.
    Up,
    /// Enter — accept selected completion (when popup open).
    Enter,
    /// Escape — dismiss popup.
    Escape,
    /// Backspace — delete char and re-trigger completion.
    Backspace,
    /// Ctrl-C — interrupt.
    CtrlC,
    /// Ctrl-Z — suspend.
    CtrlZ,
}

/// Parse raw input bytes into an InputAction.
/// Returns the action and number of bytes consumed.
pub fn classify_input(buf: &[u8]) -> (InputAction, usize) {
    if buf.is_empty() {
        return (InputAction::Passthrough(vec![]), 0);
    }

    // Tab
    if buf[0] == 0x09 {
        return (InputAction::Tab, 1);
    }

    // Escape sequences
    if buf[0] == 0x1b {
        if buf.len() == 1 {
            return (InputAction::Escape, 1);
        }

        if buf.len() >= 2 && buf[1] == b'[' {
            if buf.len() >= 3 {
                match buf[2] {
                    b'A' => return (InputAction::Up, 3),
                    b'B' => return (InputAction::Down, 3),
                    // Shift+Tab is ESC [ Z
                    b'Z' => return (InputAction::ShiftTab, 3),
                    _ => {}
                }
            }
            // Pass through other escape sequences
            let mut end = 2;
            while end < buf.len() && end < 8 {
                if buf[end] >= 0x40 && buf[end] <= 0x7E {
                    end += 1;
                    break;
                }
                end += 1;
            }
            return (InputAction::Passthrough(buf[..end].to_vec()), end);
        }

        // Alt+key or other escape sequences — passthrough
        let len = buf.len().min(2);
        return (InputAction::Passthrough(buf[..len].to_vec()), len);
    }

    // Enter
    if buf[0] == 0x0d || buf[0] == 0x0a {
        return (InputAction::Enter, 1);
    }

    // Backspace (0x7f or 0x08)
    if buf[0] == 0x7f || buf[0] == 0x08 {
        return (InputAction::Backspace, 1);
    }

    // Ctrl-C
    if buf[0] == 0x03 {
        return (InputAction::CtrlC, 1);
    }

    // Ctrl-Z
    if buf[0] == 0x1a {
        return (InputAction::CtrlZ, 1);
    }

    // Regular character (possibly multi-byte UTF-8)
    let len = if buf[0] < 0x80 {
        1
    } else if buf[0] < 0xE0 {
        2.min(buf.len())
    } else if buf[0] < 0xF0 {
        3.min(buf.len())
    } else {
        4.min(buf.len())
    };

    (InputAction::Passthrough(buf[..len].to_vec()), len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab() {
        let (action, consumed) = classify_input(&[0x09]);
        assert_eq!(action, InputAction::Tab);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn test_arrow_keys() {
        let (action, _) = classify_input(&[0x1b, b'[', b'A']);
        assert_eq!(action, InputAction::Up);
        let (action, _) = classify_input(&[0x1b, b'[', b'B']);
        assert_eq!(action, InputAction::Down);
    }

    #[test]
    fn test_escape() {
        let (action, _) = classify_input(&[0x1b]);
        assert_eq!(action, InputAction::Escape);
    }

    #[test]
    fn test_enter() {
        let (action, _) = classify_input(&[0x0d]);
        assert_eq!(action, InputAction::Enter);
    }

    #[test]
    fn test_regular_char() {
        let (action, consumed) = classify_input(b"a");
        assert_eq!(action, InputAction::Passthrough(vec![b'a']));
        assert_eq!(consumed, 1);
    }

    #[test]
    fn test_shift_tab() {
        let (action, consumed) = classify_input(&[0x1b, b'[', b'Z']);
        assert_eq!(action, InputAction::ShiftTab);
        assert_eq!(consumed, 3);
    }
}
