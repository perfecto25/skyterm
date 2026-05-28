use serde::{Deserialize, Serialize};

/// RGBA color, 0-255 per channel. Pure-data, no semantics — the *meaning* of
/// a color (default fg, ANSI red, etc.) lives in [`CellColor`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Sentinel still kept for compatibility with renderer paths that compare
    /// resolved colors. Real "default" semantics now live in [`CellColor::Default`].
    pub const DEFAULT_FG: Color = Color::rgb(0xd0, 0xd0, 0xd0);
    pub const DEFAULT_BG: Color = Color::rgb(0x10, 0x10, 0x14);

    /// Standard xterm 256-color palette generator. Used to seed [`Theme`]
    /// palettes; the runtime palette in [`Theme::palette`] is what actually
    /// gets sampled during rendering.
    pub fn xterm_256(idx: u8) -> Color {
        match idx {
            0..=15 => XTERM_DEFAULT_16[idx as usize],
            16..=231 => {
                let i = idx - 16;
                let r = i / 36;
                let g = (i / 6) % 6;
                let b = i % 6;
                fn comp(v: u8) -> u8 {
                    if v == 0 {
                        0
                    } else {
                        v * 40 + 55
                    }
                }
                Color::rgb(comp(r), comp(g), comp(b))
            }
            _ => {
                let g = (idx - 232).saturating_mul(10).saturating_add(8);
                Color::rgb(g, g, g)
            }
        }
    }
}

const XTERM_DEFAULT_16: [Color; 16] = [
    Color::rgb(0x00, 0x00, 0x00),
    Color::rgb(0xcd, 0x31, 0x31),
    Color::rgb(0x0d, 0xbc, 0x79),
    Color::rgb(0xe5, 0xe5, 0x10),
    Color::rgb(0x24, 0x72, 0xc8),
    Color::rgb(0xbc, 0x3f, 0xbc),
    Color::rgb(0x11, 0xa8, 0xcd),
    Color::rgb(0xe5, 0xe5, 0xe5),
    Color::rgb(0x66, 0x66, 0x66),
    Color::rgb(0xf1, 0x4c, 0x4c),
    Color::rgb(0x23, 0xd1, 0x8b),
    Color::rgb(0xf5, 0xf5, 0x43),
    Color::rgb(0x3b, 0x8e, 0xea),
    Color::rgb(0xd6, 0x70, 0xd6),
    Color::rgb(0x29, 0xb8, 0xdb),
    Color::rgb(0xff, 0xff, 0xff),
];

/// What the parser writes into a [`Cell`]: either a palette reference (which
/// lets the theme retint existing content when the user switches themes) or
/// a literal RGB color for truecolor sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellColor {
    /// "Use the theme's default fg/bg." Set by SGR 39 / 49 and SGR 0.
    Default,
    /// 0-255 ANSI palette index. Resolved against [`Theme::palette`] at
    /// render time, so theme changes apply to already-printed text.
    Palette(u8),
    /// Literal RGB. Used for truecolor SGR 38;2 / 48;2 sequences.
    Rgb(Color),
}

impl CellColor {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        CellColor::Rgb(Color::rgb(r, g, b))
    }
}

/// A complete color theme — what the user actually picks in Settings. The
/// renderer holds a reference to one of these and resolves every cell's
/// `CellColor` through it on each frame.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub fg: Color,
    pub bg: Color,
    pub cursor: Color,
    /// Full 256-entry palette. Indices 0-15 are the ANSI 16; 16-231 are the
    /// 6×6×6 RGB cube; 232-255 are grayscale.
    #[serde(with = "palette_serde")]
    pub palette: [Color; 256],
}

impl Theme {
    pub fn resolve_fg(&self, c: CellColor) -> Color {
        match c {
            CellColor::Default => self.fg,
            CellColor::Palette(i) => self.palette[i as usize],
            CellColor::Rgb(c) => c,
        }
    }

    pub fn resolve_bg(&self, c: CellColor) -> Color {
        match c {
            CellColor::Default => self.bg,
            CellColor::Palette(i) => self.palette[i as usize],
            CellColor::Rgb(c) => c,
        }
    }

    /// Built-in theme: deep blue background with a mint-green foreground.
    /// This is the default theme for a fresh install.
    pub fn skyterm_blue() -> Self {
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for i in 0..256u16 {
            palette[i as usize] = Color::xterm_256(i as u8);
        }
        Self {
            name: "Skyterm Blue".into(),
            fg: Color::rgb(0xad, 0xff, 0xcc),
            bg: Color::rgb(0x0c, 0x29, 0x3d),
            cursor: Color::rgb(0xaa, 0xaa, 0xaa),
            palette,
        }
    }

    /// Built-in theme: the original skyterm dark.
    pub fn skyterm_dark() -> Self {
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for i in 0..256u16 {
            palette[i as usize] = Color::xterm_256(i as u8);
        }
        Self {
            name: "Skyterm Dark".into(),
            fg: Color::rgb(0xd0, 0xd0, 0xd0),
            bg: Color::rgb(0x10, 0x10, 0x14),
            cursor: Color::rgb(0xd0, 0xd0, 0xd0),
            palette,
        }
    }

    /// Built-in theme: Solarized Dark by Ethan Schoonover.
    pub fn solarized_dark() -> Self {
        let palette = solarized_palette();
        Self {
            name: "Solarized Dark".into(),
            fg: Color::rgb(0x83, 0x94, 0x96), // base0
            bg: Color::rgb(0x00, 0x2b, 0x36), // base03
            cursor: Color::rgb(0x83, 0x94, 0x96),
            palette,
        }
    }

    /// Built-in theme: Solarized Light.
    pub fn solarized_light() -> Self {
        let palette = solarized_palette();
        Self {
            name: "Solarized Light".into(),
            fg: Color::rgb(0x65, 0x7b, 0x83), // base00
            bg: Color::rgb(0xfd, 0xf6, 0xe3), // base3
            cursor: Color::rgb(0x65, 0x7b, 0x83),
            palette,
        }
    }

    /// Built-in theme: a flat light theme for daylight reading.
    pub fn skyterm_light() -> Self {
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for i in 0..256u16 {
            palette[i as usize] = Color::xterm_256(i as u8);
        }
        Self {
            name: "Skyterm Light".into(),
            fg: Color::rgb(0x20, 0x20, 0x20),
            bg: Color::rgb(0xfa, 0xfa, 0xfa),
            cursor: Color::rgb(0x20, 0x20, 0x20),
            palette,
        }
    }

    /// All built-in presets, in the order they appear in the Settings UI.
    pub fn presets() -> Vec<Theme> {
        vec![
            Theme::skyterm_blue(),
            Theme::skyterm_dark(),
            Theme::solarized_dark(),
            Theme::solarized_light(),
            Theme::skyterm_light(),
        ]
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::skyterm_blue()
    }
}

/// Parse a Terminator-style theme file. Each `[[name]]` block becomes one
/// [`Theme`]; we honor `foreground_color`, `background_color`, `cursor_color`
/// and `palette` (the 16-color `:`-separated list). Other keys are ignored
/// — skyterm doesn't model bell behavior, background images, etc.
///
/// A block without at least `foreground_color` + `background_color` is
/// skipped silently — it's not a complete theme.
pub fn parse_terminator_themes(text: &str) -> Vec<Theme> {
    let mut themes = Vec::new();
    let mut current: Option<TerminatorBlock> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = parse_block_header(line) {
            if let Some(b) = current.take() {
                if let Some(t) = b.into_theme() {
                    themes.push(t);
                }
            }
            current = Some(TerminatorBlock::new(name));
        } else if let Some(b) = current.as_mut() {
            if let Some((key, value)) = parse_kv(line) {
                b.set(key, value);
            }
        }
    }
    if let Some(b) = current {
        if let Some(t) = b.into_theme() {
            themes.push(t);
        }
    }
    themes
}

fn parse_block_header(line: &str) -> Option<String> {
    let inner = if let Some(s) = line.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        s
    } else if let Some(s) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        s
    } else {
        return None;
    };
    Some(inner.trim().to_string())
}

fn parse_kv(line: &str) -> Option<(&str, &str)> {
    let mut parts = line.splitn(2, '=');
    let k = parts.next()?.trim();
    let v = parts.next()?.trim().trim_matches('"');
    Some((k, v))
}

struct TerminatorBlock {
    name: String,
    fg: Option<Color>,
    bg: Option<Color>,
    cursor: Option<Color>,
    palette_16: Option<[Color; 16]>,
}

impl TerminatorBlock {
    fn new(name: String) -> Self {
        Self {
            name,
            fg: None,
            bg: None,
            cursor: None,
            palette_16: None,
        }
    }

    fn set(&mut self, key: &str, value: &str) {
        match key {
            "foreground_color" => self.fg = parse_hex(value),
            "background_color" => self.bg = parse_hex(value),
            "cursor_color" => self.cursor = parse_hex(value),
            "palette" => self.palette_16 = parse_palette_16(value),
            _ => {}
        }
    }

    fn into_theme(self) -> Option<Theme> {
        let fg = self.fg?;
        let bg = self.bg?;
        let cursor = self.cursor.unwrap_or(fg);
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for i in 0..256u16 {
            palette[i as usize] = Color::xterm_256(i as u8);
        }
        if let Some(p16) = self.palette_16 {
            for (i, c) in p16.iter().enumerate() {
                palette[i] = *c;
            }
        }
        Some(Theme {
            name: self.name,
            fg,
            bg,
            cursor,
            palette,
        })
    }
}

fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_matches('"').trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::rgb(r, g, b))
}

fn parse_palette_16(s: &str) -> Option<[Color; 16]> {
    // Tolerate stray whitespace / newlines from wrapped values.
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let parts: Vec<&str> = cleaned.trim_matches('"').split(':').collect();
    if parts.len() != 16 {
        return None;
    }
    let mut out = [Color::rgb(0, 0, 0); 16];
    for (i, p) in parts.iter().enumerate() {
        out[i] = parse_hex(p)?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_terminator_solarized() {
        let text = r##"
[[solarized]]
    background_color = "#002b36"
    cursor_color = "#eee8d5"
    foreground_color = "#e6e0da"
    palette = "#073642:#dc322f:#859900:#b58900:#268bd2:#d33682:#2aa198:#eee8d5:#586e75:#cb4b16:#586e75:#657b83:#839496:#6c71c4:#93a1a1:#fdf6e3"
"##;
        let themes = parse_terminator_themes(text);
        assert_eq!(themes.len(), 1);
        let t = &themes[0];
        assert_eq!(t.name, "solarized");
        assert_eq!(t.bg, Color::rgb(0x00, 0x2b, 0x36));
        assert_eq!(t.fg, Color::rgb(0xe6, 0xe0, 0xda));
        assert_eq!(t.cursor, Color::rgb(0xee, 0xe8, 0xd5));
        assert_eq!(t.palette[0], Color::rgb(0x07, 0x36, 0x42));
        assert_eq!(t.palette[15], Color::rgb(0xfd, 0xf6, 0xe3));
    }

    #[test]
    fn skips_incomplete_block() {
        let text = "[[only-bg]]\nbackground_color = \"#000000\"\n";
        assert!(parse_terminator_themes(text).is_empty());
    }

    #[test]
    fn cursor_defaults_to_fg() {
        let text = "[[t]]\nforeground_color = \"#ffffff\"\nbackground_color = \"#000000\"\n";
        let themes = parse_terminator_themes(text);
        assert_eq!(themes[0].cursor, Color::rgb(0xff, 0xff, 0xff));
    }
}

fn solarized_palette() -> [Color; 256] {
    let mut p = [Color::rgb(0, 0, 0); 256];
    // ANSI 16, Solarized canonical values.
    let ansi = [
        Color::rgb(0x07, 0x36, 0x42), // base02 (black)
        Color::rgb(0xdc, 0x32, 0x2f), // red
        Color::rgb(0x85, 0x99, 0x00), // green
        Color::rgb(0xb5, 0x89, 0x00), // yellow
        Color::rgb(0x26, 0x8b, 0xd2), // blue
        Color::rgb(0xd3, 0x36, 0x82), // magenta
        Color::rgb(0x2a, 0xa1, 0x98), // cyan
        Color::rgb(0xee, 0xe8, 0xd5), // base2 (white)
        Color::rgb(0x00, 0x2b, 0x36), // base03 (bright black)
        Color::rgb(0xcb, 0x4b, 0x16), // orange
        Color::rgb(0x58, 0x6e, 0x75), // base01
        Color::rgb(0x65, 0x7b, 0x83), // base00
        Color::rgb(0x83, 0x94, 0x96), // base0
        Color::rgb(0x6c, 0x71, 0xc4), // violet
        Color::rgb(0x93, 0xa1, 0xa1), // base1
        Color::rgb(0xfd, 0xf6, 0xe3), // base3
    ];
    for (i, c) in ansi.iter().enumerate() {
        p[i] = *c;
    }
    // Fill 16..256 with the xterm cube/grayscale (no widely-agreed Solarized
    // 256-color extension exists; xterm defaults are a reasonable fallback).
    for i in 16..256u16 {
        p[i as usize] = Color::xterm_256(i as u8);
    }
    p
}

mod palette_serde {
    use super::Color;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(p: &[Color; 256], s: S) -> Result<S::Ok, S::Error> {
        p.as_slice().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[Color; 256], D::Error> {
        let v: Vec<Color> = Vec::deserialize(d)?;
        let mut p = [Color::rgb(0, 0, 0); 256];
        for (i, c) in v.into_iter().take(256).enumerate() {
            p[i] = c;
        }
        Ok(p)
    }
}
