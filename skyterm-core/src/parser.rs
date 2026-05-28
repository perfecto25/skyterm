use crate::grid::Grid;
use crate::theme::{CellColor, Color};

/// Wraps `vte::Parser` and drives a `Grid`. Buffers any responses the terminal
/// needs to write back to the PTY (DSR, DA, etc.); call `take_responses` after
/// each `advance` and forward them to the child.
pub struct Parser {
    inner: vte::Parser,
    responses: Vec<u8>,
    charsets: Charsets,
    bracketed_paste: bool,
}

#[derive(Default)]
struct Charsets {
    g0: Charset,
    g1: Charset,
    active: ActiveCharset,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Charset {
    #[default]
    Ascii,
    DecGraphics,
}

#[derive(Default, Clone, Copy)]
enum ActiveCharset {
    #[default]
    G0,
    G1,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            inner: vte::Parser::new(),
            responses: Vec::new(),
            charsets: Charsets::default(),
            bracketed_paste: false,
        }
    }

    /// Feed PTY bytes into the parser. The grid is mutated in place.
    pub fn advance(&mut self, grid: &mut Grid, bytes: &[u8]) {
        let mut perf = Performer {
            grid,
            responses: &mut self.responses,
            charsets: &mut self.charsets,
            bracketed_paste: &mut self.bracketed_paste,
        };
        self.inner.advance(&mut perf, bytes);
    }

    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.responses)
    }

    /// True when the foreground app has enabled DEC ?2004 (bracketed paste).
    /// Callers wrap pasted text in `\e[200~`…`\e[201~` when this is set so the
    /// app can distinguish a paste from typed input.
    pub fn bracketed_paste(&self) -> bool {
        self.bracketed_paste
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

struct Performer<'a> {
    grid: &'a mut Grid,
    responses: &'a mut Vec<u8>,
    charsets: &'a mut Charsets,
    bracketed_paste: &'a mut bool,
}

/// Translate ASCII 0x60..0x7E through the DEC Special Graphics character set.
fn dec_graphics_map(c: char) -> char {
    match c {
        '_' => ' ',
        '`' => '◆',
        'a' => '▒',
        'f' => '°',
        'g' => '±',
        'h' => '␤',
        'i' => '␋',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        _ => c,
    }
}

impl<'a> Performer<'a> {
    fn handle_sgr(&mut self, params: &vte::Params) {
        let mut flat: Vec<u16> = Vec::with_capacity(8);
        for p in params.iter() {
            for &sp in p {
                flat.push(sp);
            }
        }
        if flat.is_empty() {
            self.grid.reset_attrs();
            return;
        }
        let mut i = 0;
        while i < flat.len() {
            let code = flat[i];
            match code {
                0 => self.grid.reset_attrs(),
                1 => self.grid.set_bold(true),
                7 => self.grid.set_reverse(true),
                22 => self.grid.set_bold(false),
                27 => self.grid.set_reverse(false),
                30..=37 => self.grid.set_fg(CellColor::Palette((code - 30) as u8)),
                39 => self.grid.set_fg(CellColor::Default),
                40..=47 => self.grid.set_bg(CellColor::Palette((code - 40) as u8)),
                49 => self.grid.set_bg(CellColor::Default),
                90..=97 => self.grid.set_fg(CellColor::Palette((code - 90 + 8) as u8)),
                100..=107 => self.grid.set_bg(CellColor::Palette((code - 100 + 8) as u8)),
                38 => {
                    if let Some(advance) = parse_extended_color(&flat[i + 1..], |c| {
                        self.grid.set_fg(c)
                    }) {
                        i += advance;
                    }
                }
                48 => {
                    if let Some(advance) = parse_extended_color(&flat[i + 1..], |c| {
                        self.grid.set_bg(c)
                    }) {
                        i += advance;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn handle_dec_private(&mut self, params: &vte::Params, action: char) {
        let set = match action {
            'h' => true,
            'l' => false,
            _ => return,
        };
        for p in params.iter() {
            let Some(&code) = p.first() else { continue };
            match code {
                25 => self.grid.set_cursor_visible(set),
                47 | 1047 => {
                    if set {
                        self.grid.enter_alt_screen();
                    } else {
                        self.grid.exit_alt_screen();
                    }
                }
                1048 => {
                    if set {
                        self.grid.save_cursor();
                    } else {
                        self.grid.restore_cursor();
                    }
                }
                1049 => {
                    if set {
                        self.grid.save_cursor();
                        self.grid.enter_alt_screen();
                    } else {
                        self.grid.exit_alt_screen();
                        self.grid.restore_cursor();
                    }
                }
                2004 => *self.bracketed_paste = set,
                _ => {
                    // Many other modes exist (?7 autowrap, ?1000 mouse, …).
                    // Wired in later as needed.
                }
            }
        }
    }
}

fn parse_extended_color<F: FnOnce(CellColor)>(rest: &[u16], set: F) -> Option<usize> {
    match rest.first()? {
        5 => {
            let idx = *rest.get(1)? as u8;
            set(CellColor::Palette(idx));
            Some(2)
        }
        2 => {
            let (r, g, b) = if rest.len() >= 5 && rest[1] == 0 {
                (rest.get(2)?, rest.get(3)?, rest.get(4)?)
            } else if rest.len() >= 4 {
                (rest.get(1)?, rest.get(2)?, rest.get(3)?)
            } else {
                return None;
            };
            set(CellColor::Rgb(Color::rgb(*r as u8, *g as u8, *b as u8)));
            Some(if rest.len() >= 5 && rest[1] == 0 { 4 } else { 3 })
        }
        _ => None,
    }
}

impl<'a> vte::Perform for Performer<'a> {
    fn print(&mut self, c: char) {
        let active = match self.charsets.active {
            ActiveCharset::G0 => self.charsets.g0,
            ActiveCharset::G1 => self.charsets.g1,
        };
        let translated = match active {
            Charset::Ascii => c,
            Charset::DecGraphics => dec_graphics_map(c),
        };
        self.grid.put_char(translated);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => {}
            0x08 => self.grid.backspace(),
            0x09 => self.grid.tab(),
            0x0a | 0x0b | 0x0c => self.grid.linefeed(),
            0x0d => self.grid.carriage_return(),
            0x0e => self.charsets.active = ActiveCharset::G1, // SO
            0x0f => self.charsets.active = ActiveCharset::G0, // SI
            _ => {}
        }
    }

    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // M2+: window title via OSC 0/2. For now ignore.
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        if intermediates == [b'?'] {
            self.handle_dec_private(params, action);
            return;
        }
        if !intermediates.is_empty() {
            // Other CSI families (`>`, `=`, `!`, space, etc.) — ignore.
            return;
        }

        let first = params.iter().next().and_then(|p| p.first().copied());
        let second = params.iter().nth(1).and_then(|p| p.first().copied());
        let nz = |v: Option<u16>, default: u16| {
            let v = v.unwrap_or(0);
            if v == 0 {
                default as usize
            } else {
                v as usize
            }
        };

        match action {
            'A' => self.grid.cursor_up(nz(first, 1)),
            'B' | 'e' => self.grid.cursor_down(nz(first, 1)),
            'C' | 'a' => self.grid.cursor_forward(nz(first, 1)),
            'D' => self.grid.cursor_back(nz(first, 1)),
            'E' => {
                self.grid.cursor_down(nz(first, 1));
                self.grid.carriage_return();
            }
            'F' => {
                self.grid.cursor_up(nz(first, 1));
                self.grid.carriage_return();
            }
            'G' | '`' => self.grid.cursor_to_column(nz(first, 1)),
            'd' => self.grid.cursor_to_row(nz(first, 1)),
            'H' | 'f' => {
                self.grid.set_cursor(nz(first, 1), nz(second, 1));
            }
            'J' => self.grid.erase_in_display(first.unwrap_or(0)),
            'K' => self.grid.erase_in_line(first.unwrap_or(0)),
            'P' => self.grid.delete_chars(nz(first, 1)),
            '@' => self.grid.insert_chars(nz(first, 1)),
            'X' => self.grid.erase_chars(nz(first, 1)),
            'L' => self.grid.insert_lines(nz(first, 1)),
            'M' => self.grid.delete_lines(nz(first, 1)),
            'S' => self.grid.scroll_up(nz(first, 1)),
            'T' => self.grid.scroll_down(nz(first, 1)),
            'r' => self
                .grid
                .set_scroll_region(first.unwrap_or(0) as usize, second.unwrap_or(0) as usize),
            'm' => self.handle_sgr(params),
            's' => self.grid.save_cursor(),
            'u' => self.grid.restore_cursor(),
            'n' => {
                if first == Some(6) {
                    let (row, col) = self.grid.cursor();
                    let r = format!("\x1b[{};{}R", row + 1, col + 1);
                    self.responses.extend_from_slice(r.as_bytes());
                }
            }
            'c' => {
                // Primary device attributes — claim VT100 + advanced video.
                self.responses.extend_from_slice(b"\x1b[?1;2c");
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if intermediates.is_empty() {
            match byte {
                b'7' => self.grid.save_cursor(),
                b'8' => self.grid.restore_cursor(),
                _ => {}
            }
            return;
        }
        // Designate G0 / G1 character sets. ESC ( <c> sets G0; ESC ) <c> sets G1.
        if intermediates.len() == 1 && (intermediates[0] == b'(' || intermediates[0] == b')') {
            let charset = match byte {
                b'0' => Charset::DecGraphics,
                // Everything else we treat as ASCII (B = US ASCII, A = UK, etc.).
                _ => Charset::Ascii,
            };
            if intermediates[0] == b'(' {
                self.charsets.g0 = charset;
            } else {
                self.charsets.g1 = charset;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_ascii() {
        let mut g = Grid::new(20, 2);
        let mut p = Parser::new();
        p.advance(&mut g, b"hello");
        let row: String = g.row(0)[..5].iter().map(|c| c.ch).collect();
        assert_eq!(row, "hello");
    }

    #[test]
    fn handles_cr_lf() {
        let mut g = Grid::new(10, 3);
        let mut p = Parser::new();
        p.advance(&mut g, b"abc\r\nxyz");
        assert_eq!(
            &g.row(0)[..3].iter().map(|c| c.ch).collect::<String>(),
            "abc"
        );
        assert_eq!(
            &g.row(1)[..3].iter().map(|c| c.ch).collect::<String>(),
            "xyz"
        );
    }

    #[test]
    fn cursor_back_then_overwrite() {
        let mut g = Grid::new(10, 1);
        let mut p = Parser::new();
        p.advance(&mut g, b"hello\x1b[4DOO");
        let s: String = g.row(0)[..5].iter().map(|c| c.ch).collect();
        assert_eq!(s, "hOOlo");
    }

    #[test]
    fn erase_to_end_of_line() {
        let mut g = Grid::new(10, 1);
        let mut p = Parser::new();
        p.advance(&mut g, b"abcdef\x1b[4G\x1b[K");
        let s: String = g.row(0).iter().map(|c| c.ch).collect();
        assert_eq!(s, "abc       ");
    }

    #[test]
    fn sgr_truecolor_fg() {
        let mut g = Grid::new(4, 1);
        let mut p = Parser::new();
        p.advance(&mut g, b"\x1b[38;2;255;128;64mX");
        let cell = g.cell(0, 0);
        assert_eq!(cell.ch, 'X');
        assert_eq!(cell.fg, CellColor::Rgb(Color::rgb(255, 128, 64)));
    }

    #[test]
    fn sgr_256color_bg_and_reset() {
        let mut g = Grid::new(4, 1);
        let mut p = Parser::new();
        p.advance(&mut g, b"\x1b[48;5;9mA\x1b[0mB");
        assert_eq!(g.cell(0, 0).bg, CellColor::Palette(9));
        assert_eq!(g.cell(0, 1).bg, CellColor::Default);
    }

    #[test]
    fn dec_private_mode_swallowed() {
        let mut g = Grid::new(4, 1);
        let mut p = Parser::new();
        p.advance(&mut g, b"\x1b[?2004hX");
        assert_eq!(g.cell(0, 0).ch, 'X');
    }

    #[test]
    fn cup_then_print() {
        let mut g = Grid::new(10, 5);
        let mut p = Parser::new();
        p.advance(&mut g, b"\x1b[3;5HX");
        assert_eq!(g.cell(2, 4).ch, 'X');
    }

    #[test]
    fn cursor_visibility_toggles() {
        let mut g = Grid::new(4, 1);
        let mut p = Parser::new();
        assert!(g.cursor_visible());
        p.advance(&mut g, b"\x1b[?25l");
        assert!(!g.cursor_visible());
        p.advance(&mut g, b"\x1b[?25h");
        assert!(g.cursor_visible());
    }

    #[test]
    fn alt_screen_swap_round_trip() {
        let mut g = Grid::new(4, 1);
        let mut p = Parser::new();
        p.advance(&mut g, b"main");
        // Enter alt — main should be saved, alt is blank
        p.advance(&mut g, b"\x1b[?1049h");
        assert!(g.is_alt_screen());
        assert_eq!(g.row(0).iter().map(|c| c.ch).collect::<String>(), "    ");
        p.advance(&mut g, b"alt!");
        assert_eq!(g.row(0).iter().map(|c| c.ch).collect::<String>(), "alt!");
        // Exit — main content comes back
        p.advance(&mut g, b"\x1b[?1049l");
        assert!(!g.is_alt_screen());
        assert_eq!(g.row(0).iter().map(|c| c.ch).collect::<String>(), "main");
    }

    #[test]
    fn alt_screen_does_not_pollute_scrollback() {
        let mut g = Grid::new(4, 2);
        let mut p = Parser::new();
        p.advance(&mut g, b"\x1b[?1049h");
        // Cause lots of linefeeds in alt screen — none should hit scrollback.
        for _ in 0..50 {
            p.advance(&mut g, b"x\r\n");
        }
        assert_eq!(g.scrollback_len(), 0);
    }

    #[test]
    fn dsr_cursor_position_response() {
        let mut g = Grid::new(20, 5);
        let mut p = Parser::new();
        p.advance(&mut g, b"\x1b[3;5H\x1b[6n");
        let resp = p.take_responses();
        assert_eq!(resp, b"\x1b[3;5R");
    }

    #[test]
    fn primary_device_attributes_response() {
        let mut g = Grid::new(4, 1);
        let mut p = Parser::new();
        p.advance(&mut g, b"\x1b[c");
        let resp = p.take_responses();
        assert_eq!(resp, b"\x1b[?1;2c");
    }

    #[test]
    fn dec_graphics_translates_box_drawing() {
        let mut g = Grid::new(10, 1);
        let mut p = Parser::new();
        // Switch G0 to DEC graphics, print 'l q q k', switch back.
        p.advance(&mut g, b"\x1b(0lqqk\x1b(B");
        let s: String = g.row(0)[..4].iter().map(|c| c.ch).collect();
        assert_eq!(s, "┌──┐");
    }

    #[test]
    fn so_si_shift_between_g0_g1() {
        let mut g = Grid::new(8, 1);
        let mut p = Parser::new();
        // Designate G1 = DEC graphics, switch active to G1 via SO, print, SI back.
        p.advance(&mut g, b"\x1b)0X\x0elqj\x0fY");
        let s: String = g.row(0)[..5].iter().map(|c| c.ch).collect();
        assert_eq!(s, "X┌─┘Y");
    }

    #[test]
    fn decsc_decrc_save_restore_cursor() {
        let mut g = Grid::new(10, 2);
        let mut p = Parser::new();
        // Move to (1,5), save, move elsewhere, restore.
        p.advance(&mut g, b"\x1b[2;5H\x1b7\x1b[1;1H\x1b8");
        assert_eq!(g.cursor(), (1, 4)); // 1-based 2,5 -> 0-based (1,4)
    }
}
