use gtk4::gdk;

/// Translate a GDK key event into bytes for the PTY.
///
/// `app_cursor` is the grid's DECCKM state: when true the arrow/Home/End keys
/// transmit in application mode (`ESC O x`) rather than normal mode (`ESC [ x`).
/// ncurses apps (htop, vim, less) enable DECCKM via `keypad()` and won't match
/// arrows sent in the wrong mode — so without this, Down/Up do nothing in htop.
pub fn encode_key(keyval: gdk::Key, modifiers: gdk::ModifierType, app_cursor: bool) -> Vec<u8> {
    let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
    // SS3 (`ESC O`) in application mode, CSI (`ESC [`) in normal mode.
    let cursor = |final_byte: u8| -> Vec<u8> {
        if app_cursor {
            vec![0x1b, b'O', final_byte]
        } else {
            vec![0x1b, b'[', final_byte]
        }
    };

    match keyval {
        gdk::Key::Return | gdk::Key::KP_Enter => return vec![b'\r'],
        gdk::Key::BackSpace => return vec![0x7f],
        gdk::Key::Tab => return vec![b'\t'],
        gdk::Key::Escape => return vec![0x1b],
        gdk::Key::Up => return cursor(b'A'),
        gdk::Key::Down => return cursor(b'B'),
        gdk::Key::Right => return cursor(b'C'),
        gdk::Key::Left => return cursor(b'D'),
        gdk::Key::Home | gdk::Key::KP_Home => return cursor(b'H'),
        gdk::Key::End | gdk::Key::KP_End => return cursor(b'F'),
        gdk::Key::Page_Up | gdk::Key::KP_Page_Up => return b"\x1b[5~".to_vec(),
        gdk::Key::Page_Down | gdk::Key::KP_Page_Down => return b"\x1b[6~".to_vec(),
        gdk::Key::Insert | gdk::Key::KP_Insert => return b"\x1b[2~".to_vec(),
        gdk::Key::Delete | gdk::Key::KP_Delete => return b"\x1b[3~".to_vec(),
        // Function keys (xterm encodings; F1–F4 use SS3, the rest CSI ~).
        // ncurses apps match these against terminfo kf1…kf12 — e.g. htop's
        // F3 search needs `ESC O R`.
        gdk::Key::F1 => return b"\x1bOP".to_vec(),
        gdk::Key::F2 => return b"\x1bOQ".to_vec(),
        gdk::Key::F3 => return b"\x1bOR".to_vec(),
        gdk::Key::F4 => return b"\x1bOS".to_vec(),
        gdk::Key::F5 => return b"\x1b[15~".to_vec(),
        gdk::Key::F6 => return b"\x1b[17~".to_vec(),
        gdk::Key::F7 => return b"\x1b[18~".to_vec(),
        gdk::Key::F8 => return b"\x1b[19~".to_vec(),
        gdk::Key::F9 => return b"\x1b[20~".to_vec(),
        gdk::Key::F10 => return b"\x1b[21~".to_vec(),
        gdk::Key::F11 => return b"\x1b[23~".to_vec(),
        gdk::Key::F12 => return b"\x1b[24~".to_vec(),
        _ => {}
    }

    if let Some(ch) = keyval.to_unicode() {
        if ctrl && ch.is_ascii_alphabetic() {
            return vec![(ch.to_ascii_lowercase() as u8) & 0x1f];
        }
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        return s.as_bytes().to_vec();
    }

    Vec::new()
}
