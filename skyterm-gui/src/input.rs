use gtk4::gdk;

/// Translate a GDK key event into bytes for the PTY.
///
/// M1 scope: ASCII text + a handful of control keys. CSI escape sequences for
/// arrow keys, F-keys, modifiers etc. arrive in M2.
pub fn encode_key(keyval: gdk::Key, modifiers: gdk::ModifierType) -> Vec<u8> {
    let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);

    match keyval {
        gdk::Key::Return | gdk::Key::KP_Enter => return vec![b'\r'],
        gdk::Key::BackSpace => return vec![0x7f],
        gdk::Key::Tab => return vec![b'\t'],
        gdk::Key::Escape => return vec![0x1b],
        gdk::Key::Up => return b"\x1b[A".to_vec(),
        gdk::Key::Down => return b"\x1b[B".to_vec(),
        gdk::Key::Right => return b"\x1b[C".to_vec(),
        gdk::Key::Left => return b"\x1b[D".to_vec(),
        gdk::Key::Home | gdk::Key::KP_Home => return b"\x1bOH".to_vec(),
        gdk::Key::End | gdk::Key::KP_End => return b"\x1bOF".to_vec(),
        gdk::Key::Page_Up | gdk::Key::KP_Page_Up => return b"\x1b[5~".to_vec(),
        gdk::Key::Page_Down | gdk::Key::KP_Page_Down => return b"\x1b[6~".to_vec(),
        gdk::Key::Insert | gdk::Key::KP_Insert => return b"\x1b[2~".to_vec(),
        gdk::Key::Delete | gdk::Key::KP_Delete => return b"\x1b[3~".to_vec(),
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
