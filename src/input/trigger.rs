//! Key classification for the completion state machine.
/// Actions that the input processor can produce.
#[derive(Debug, Clone, PartialEq)]
pub enum InputAction {
    /// Normal character — passthrough to PTY.
    Passthrough,
    /// Tab pressed — activate/cycle completion.
    Tab,
    /// Shift+Tab — cycle completion backwards.
    ShiftTab,
    /// Arrow down — select next item in popup.
    Down,
    /// Arrow up — select previous item in popup.
    Up,
    /// Arrow left — move cursor left.
    Left,
    /// Arrow right — move cursor right.
    Right,
    /// Home / Ctrl-A — move cursor to line start.
    Home,
    /// End / Ctrl-E — move cursor to line end.
    End,
    /// Delete the character under the cursor.
    Delete,
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
    /// Ctrl-J — move down in popup (or passthrough as LF in passthrough mode).
    CtrlJ,
    /// Ctrl-K — move up in popup (or passthrough in passthrough mode).
    CtrlK,
    /// Ctrl-W or Option+Backspace — kill last word.
    KillWord,
    /// Ctrl-U — kill entire line.
    KillLine,
}

/// Parse raw input bytes into an InputAction.
/// Returns the action and number of bytes consumed.
pub fn classify_input(buf: &[u8]) -> (InputAction, usize) {
    if buf.is_empty() {
        return (InputAction::Passthrough, 0);
    }

    // Tab
    if buf[0] == 0x09 {
        return (InputAction::Tab, 1);
    }

    // Ctrl-W — kill last word
    if buf[0] == 0x17 {
        return (InputAction::KillWord, 1);
    }

    // Ctrl-A — line start
    if buf[0] == 0x01 {
        return (InputAction::Home, 1);
    }

    // Ctrl-E — line end
    if buf[0] == 0x05 {
        return (InputAction::End, 1);
    }

    // Ctrl-U — kill entire line
    if buf[0] == 0x15 {
        return (InputAction::KillLine, 1);
    }

    // Escape sequences
    if buf[0] == 0x1b {
        if buf.len() == 1 {
            return (InputAction::Escape, 1);
        }

        // Option+Backspace (macOS): ESC + DEL — kill last word
        if buf[1] == 0x7f {
            return (InputAction::KillWord, 2);
        }

        if buf.len() >= 2 && buf[1] == b'[' {
            if buf.len() >= 3 {
                match buf[2] {
                    b'A' => return (InputAction::Up, 3),
                    b'B' => return (InputAction::Down, 3),
                    b'C' => return (InputAction::Right, 3),
                    b'D' => return (InputAction::Left, 3),
                    b'H' => return (InputAction::Home, 3),
                    b'F' => return (InputAction::End, 3),
                    // Shift+Tab is ESC [ Z
                    b'Z' => return (InputAction::ShiftTab, 3),
                    b'1' if buf.len() >= 4 && buf[3] == b'~' => return (InputAction::Home, 4),
                    b'4' if buf.len() >= 4 && buf[3] == b'~' => return (InputAction::End, 4),
                    b'3' if buf.len() >= 4 && buf[3] == b'~' => return (InputAction::Delete, 4),
                    _ => {}
                }
            }
            // Pass through other CSI escape sequences.
            // CSI sequences: ESC [ (parameter bytes 0x30-0x3F)* (intermediate bytes 0x20-0x2F)* (final byte 0x40-0x7E)
            // Mouse sequences (SGR: ESC[<...M/m) can be very long, so don't cap the scan length.
            let mut end = 2;
            while end < buf.len() {
                if buf[end] >= 0x40 && buf[end] <= 0x7E {
                    end += 1;
                    break;
                }
                end += 1;
            }
            return (InputAction::Passthrough, end);
        }

        // Alt+key or other escape sequences — passthrough
        let len = buf.len().min(2);
        return (InputAction::Passthrough, len);
    }

    // Enter (CR only; LF/0x0a is Ctrl-J and handled separately)
    if buf[0] == 0x0d {
        return (InputAction::Enter, 1);
    }

    // Ctrl-J (LF) — navigate down in popup
    if buf[0] == 0x0a {
        return (InputAction::CtrlJ, 1);
    }

    // Ctrl-K (VT) — navigate up in popup
    if buf[0] == 0x0b {
        return (InputAction::CtrlK, 1);
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

    (InputAction::Passthrough, len)
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
        let (action, _) = classify_input(&[0x1b, b'[', b'C']);
        assert_eq!(action, InputAction::Right);
        let (action, _) = classify_input(&[0x1b, b'[', b'D']);
        assert_eq!(action, InputAction::Left);
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
        assert_eq!(action, InputAction::Passthrough);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn test_shift_tab() {
        let (action, consumed) = classify_input(&[0x1b, b'[', b'Z']);
        assert_eq!(action, InputAction::ShiftTab);
        assert_eq!(consumed, 3);
    }

    #[test]
    fn test_delete_and_home_end() {
        let (action, consumed) = classify_input(&[0x1b, b'[', b'3', b'~']);
        assert_eq!(action, InputAction::Delete);
        assert_eq!(consumed, 4);

        let (action, _) = classify_input(&[0x01]);
        assert_eq!(action, InputAction::Home);
        let (action, _) = classify_input(&[0x05]);
        assert_eq!(action, InputAction::End);
    }
}
