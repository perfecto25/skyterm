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

    /// Built-in theme: Dracula by Zeno Rocha.
    pub fn skyterm_dracula() -> Self {
        let ansi: [Color; 16] = [
            Color::rgb(0x21, 0x22, 0x2c), // black
            Color::rgb(0xff, 0x55, 0x55), // red
            Color::rgb(0x50, 0xfa, 0x7b), // green
            Color::rgb(0xf1, 0xfa, 0x8c), // yellow
            Color::rgb(0xbd, 0x93, 0xf9), // blue (violet)
            Color::rgb(0xff, 0x79, 0xc6), // magenta
            Color::rgb(0x8b, 0xe9, 0xfd), // cyan
            Color::rgb(0xf8, 0xf8, 0xf2), // white
            Color::rgb(0x62, 0x72, 0xa4), // bright black
            Color::rgb(0xff, 0x6e, 0x6e), // bright red
            Color::rgb(0x69, 0xff, 0x94), // bright green
            Color::rgb(0xff, 0xff, 0xa5), // bright yellow
            Color::rgb(0xd6, 0xac, 0xff), // bright blue
            Color::rgb(0xff, 0x92, 0xdf), // bright magenta
            Color::rgb(0xa4, 0xff, 0xff), // bright cyan
            Color::rgb(0xff, 0xff, 0xff), // bright white
        ];
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for (i, c) in ansi.iter().enumerate() { palette[i] = *c; }
        for i in 16..256u16 { palette[i as usize] = Color::xterm_256(i as u8); }
        Self {
            name: "Skyterm Dracula".into(),
            fg: Color::rgb(0xf8, 0xf8, 0xf2),
            bg: Color::rgb(0x28, 0x2a, 0x36),
            cursor: Color::rgb(0xf8, 0xf8, 0xf2),
            palette,
        }
    }

    /// Built-in theme: deep purple aesthetic.
    pub fn skyterm_purple() -> Self {
        let ansi: [Color; 16] = [
            Color::rgb(0x15, 0x08, 0x26), // black
            Color::rgb(0xff, 0x33, 0x66), // red
            Color::rgb(0x5a, 0xf7, 0x8e), // green
            Color::rgb(0xf3, 0xf9, 0x9d), // yellow
            Color::rgb(0x57, 0xc7, 0xff), // blue
            Color::rgb(0xff, 0x6a, 0xc1), // magenta
            Color::rgb(0x9a, 0xed, 0xfe), // cyan
            Color::rgb(0xd1, 0xc4, 0xe9), // white
            Color::rgb(0x4a, 0x35, 0x60), // bright black
            Color::rgb(0xff, 0x6b, 0x6b), // bright red
            Color::rgb(0x69, 0xff, 0x94), // bright green
            Color::rgb(0xff, 0xff, 0xa5), // bright yellow
            Color::rgb(0xa2, 0x9b, 0xfe), // bright blue
            Color::rgb(0xfd, 0x79, 0xa8), // bright magenta
            Color::rgb(0x74, 0xb9, 0xff), // bright cyan
            Color::rgb(0xff, 0xff, 0xff), // bright white
        ];
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for (i, c) in ansi.iter().enumerate() { palette[i] = *c; }
        for i in 16..256u16 { palette[i as usize] = Color::xterm_256(i as u8); }
        Self {
            name: "Skyterm Purple".into(),
            fg: Color::rgb(0xe0, 0xd0, 0xff),
            bg: Color::rgb(0x1a, 0x0d, 0x2e),
            cursor: Color::rgb(0xbf, 0x9f, 0xff),
            palette,
        }
    }

    /// Built-in theme: Atom One Light.
    pub fn skyterm_atom() -> Self {
        let ansi: [Color; 16] = [
            Color::rgb(0x38, 0x3a, 0x42), // black
            Color::rgb(0xe4, 0x56, 0x49), // red
            Color::rgb(0x50, 0xa1, 0x4f), // green
            Color::rgb(0xc1, 0x84, 0x01), // yellow
            Color::rgb(0x40, 0x78, 0xf2), // blue
            Color::rgb(0xa6, 0x26, 0xa4), // magenta
            Color::rgb(0x01, 0x84, 0xbc), // cyan
            Color::rgb(0xa0, 0xa1, 0xa7), // white
            Color::rgb(0x69, 0x6c, 0x77), // bright black
            Color::rgb(0xe4, 0x56, 0x49), // bright red
            Color::rgb(0x50, 0xa1, 0x4f), // bright green
            Color::rgb(0xc1, 0x84, 0x01), // bright yellow
            Color::rgb(0x40, 0x78, 0xf2), // bright blue
            Color::rgb(0xa6, 0x26, 0xa4), // bright magenta
            Color::rgb(0x01, 0x84, 0xbc), // bright cyan
            Color::rgb(0xff, 0xff, 0xff), // bright white
        ];
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for (i, c) in ansi.iter().enumerate() { palette[i] = *c; }
        for i in 16..256u16 { palette[i as usize] = Color::xterm_256(i as u8); }
        Self {
            name: "Skyterm Atom".into(),
            fg: Color::rgb(0x38, 0x3a, 0x42),
            bg: Color::rgb(0xfa, 0xfa, 0xfa),
            cursor: Color::rgb(0x52, 0x6f, 0xff),
            palette,
        }
    }

    /// Built-in theme: neutral gray — muted, low-contrast palette for low-glare environments.
    pub fn skyterm_gray() -> Self {
        let ansi: [Color; 16] = [
            Color::rgb(0x1e, 0x1e, 0x1e), // black
            Color::rgb(0xc0, 0x39, 0x2b), // red
            Color::rgb(0x27, 0xae, 0x60), // green
            Color::rgb(0xd4, 0x88, 0x0e), // yellow (amber)
            Color::rgb(0x29, 0x80, 0xb9), // blue
            Color::rgb(0x8e, 0x44, 0xad), // magenta
            Color::rgb(0x16, 0xa0, 0x85), // cyan
            Color::rgb(0xbd, 0xc3, 0xc7), // white
            Color::rgb(0x7f, 0x8c, 0x8d), // bright black
            Color::rgb(0xe7, 0x4c, 0x3c), // bright red
            Color::rgb(0x2e, 0xcc, 0x71), // bright green
            Color::rgb(0xf3, 0x9c, 0x12), // bright yellow
            Color::rgb(0x34, 0x98, 0xdb), // bright blue
            Color::rgb(0x9b, 0x59, 0xb6), // bright magenta
            Color::rgb(0x1a, 0xbc, 0x9c), // bright cyan
            Color::rgb(0xec, 0xf0, 0xf1), // bright white
        ];
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for (i, c) in ansi.iter().enumerate() { palette[i] = *c; }
        for i in 16..256u16 { palette[i as usize] = Color::xterm_256(i as u8); }
        Self {
            name: "Skyterm Gray".into(),
            fg: Color::rgb(0x2c, 0x2c, 0x2c),
            bg: Color::rgb(0xee, 0xee, 0xee),
            cursor: Color::rgb(0x55, 0x55, 0x55),
            palette,
        }
    }

    /// Built-in theme: warm sandy beige — earthy palette for low-glare daylight use.
    pub fn skyterm_beige() -> Self {
        let ansi: [Color; 16] = [
            Color::rgb(0x2c, 0x1e, 0x0f), // black (dark brown)
            Color::rgb(0xb0, 0x3a, 0x2e), // red (warm crimson)
            Color::rgb(0x5d, 0x7a, 0x1f), // green (olive)
            Color::rgb(0xb7, 0x86, 0x0b), // yellow (golden)
            Color::rgb(0x2e, 0x6d, 0xa4), // blue (steel)
            Color::rgb(0x8e, 0x44, 0x7a), // magenta (dusty rose)
            Color::rgb(0x1a, 0x7d, 0x73), // cyan (teal)
            Color::rgb(0xd4, 0xc9, 0xb0), // white (light tan)
            Color::rgb(0x7a, 0x67, 0x52), // bright black (warm gray)
            Color::rgb(0xe0, 0x4e, 0x3b), // bright red
            Color::rgb(0x7d, 0xb5, 0x2f), // bright green
            Color::rgb(0xe6, 0xa8, 0x17), // bright yellow
            Color::rgb(0x4a, 0x90, 0xc4), // bright blue
            Color::rgb(0xb0, 0x6e, 0xc0), // bright magenta
            Color::rgb(0x2a, 0x9e, 0x93), // bright cyan
            Color::rgb(0xf5, 0xf0, 0xe8), // bright white
        ];
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for (i, c) in ansi.iter().enumerate() { palette[i] = *c; }
        for i in 16..256u16 { palette[i as usize] = Color::xterm_256(i as u8); }
        Self {
            name: "Skyterm Beige".into(),
            fg: Color::rgb(0x3a, 0x2e, 0x20),
            bg: Color::rgb(0xf2, 0xe8, 0xd5),
            cursor: Color::rgb(0x8c, 0x6d, 0x3f),
            palette,
        }
    }

    /// Built-in theme: hunter green — deep forest aesthetic with naturalistic ANSI colors.
    pub fn skyterm_hunter() -> Self {
        let ansi: [Color; 16] = [
            Color::rgb(0x0d, 0x1a, 0x10), // black (near-black forest)
            Color::rgb(0xcc, 0x44, 0x44), // red (muted crimson)
            Color::rgb(0x5a, 0xb4, 0x5a), // green (medium forest)
            Color::rgb(0xc8, 0xa0, 0x30), // yellow (golden)
            Color::rgb(0x44, 0x88, 0xbb), // blue (steel)
            Color::rgb(0x99, 0x66, 0xaa), // magenta (muted purple)
            Color::rgb(0x44, 0xaa, 0x88), // cyan (forest teal)
            Color::rgb(0xa8, 0xc8, 0xa8), // white (sage)
            Color::rgb(0x3a, 0x5a, 0x3a), // bright black (dark moss)
            Color::rgb(0xe0, 0x55, 0x55), // bright red
            Color::rgb(0x77, 0xcc, 0x77), // bright green
            Color::rgb(0xdd, 0xb8, 0x40), // bright yellow
            Color::rgb(0x66, 0xaa, 0xdd), // bright blue
            Color::rgb(0xbb, 0x88, 0xcc), // bright magenta
            Color::rgb(0x55, 0xcc, 0x99), // bright cyan
            Color::rgb(0xd8, 0xea, 0xd8), // bright white (pale sage)
        ];
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for (i, c) in ansi.iter().enumerate() { palette[i] = *c; }
        for i in 16..256u16 { palette[i as usize] = Color::xterm_256(i as u8); }
        Self {
            name: "Skyterm Hunter".into(),
            fg: Color::rgb(0xb8, 0xd4, 0xb8),
            bg: Color::rgb(0x1a, 0x2a, 0x1e),
            cursor: Color::rgb(0x6d, 0xbf, 0x6d),
            palette,
        }
    }

    /// Built-in theme: Cobalt Neon — deep cobalt blue with electric neon accents.
    pub fn cobalt_neon() -> Self {
        let ansi: [Color; 16] = [
            Color::rgb(0x0a, 0x0f, 0x1e), // black
            Color::rgb(0xff, 0x2d, 0x5e), // red (neon hot pink)
            Color::rgb(0x00, 0xff, 0x88), // green (neon mint)
            Color::rgb(0xff, 0xe6, 0x00), // yellow (electric)
            Color::rgb(0x3d, 0x9a, 0xff), // blue (electric cobalt)
            Color::rgb(0xff, 0x00, 0xcc), // magenta (neon fuchsia)
            Color::rgb(0x00, 0xe5, 0xff), // cyan (neon electric)
            Color::rgb(0xc8, 0xd8, 0xf0), // white (light blue-white)
            Color::rgb(0x2a, 0x3a, 0x5a), // bright black (dark cobalt)
            Color::rgb(0xff, 0x50, 0x80), // bright red
            Color::rgb(0x39, 0xff, 0x95), // bright green
            Color::rgb(0xff, 0xee, 0x44), // bright yellow
            Color::rgb(0x66, 0xb2, 0xff), // bright blue
            Color::rgb(0xff, 0x44, 0xdd), // bright magenta
            Color::rgb(0x44, 0xee, 0xff), // bright cyan
            Color::rgb(0xff, 0xff, 0xff), // bright white
        ];
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for (i, c) in ansi.iter().enumerate() { palette[i] = *c; }
        for i in 16..256u16 { palette[i as usize] = Color::xterm_256(i as u8); }
        Self {
            name: "Cobalt Neon".into(),
            fg: Color::rgb(0xc8, 0xe0, 0xff),
            bg: Color::rgb(0x0d, 0x1f, 0x3c),
            cursor: Color::rgb(0x00, 0xe5, 0xff),
            palette,
        }
    }

    /// Built-in theme: GitHub Light — canonical GitHub UI palette.
    pub fn github_light() -> Self {
        let ansi: [Color; 16] = [
            Color::rgb(0x24, 0x29, 0x2e), // black
            Color::rgb(0xd7, 0x3a, 0x49), // red
            Color::rgb(0x22, 0x86, 0x3a), // green
            Color::rgb(0xb0, 0x88, 0x00), // yellow (amber)
            Color::rgb(0x00, 0x5c, 0xc5), // blue
            Color::rgb(0x6f, 0x42, 0xc1), // magenta (purple)
            Color::rgb(0x1b, 0x7c, 0x83), // cyan
            Color::rgb(0x6a, 0x73, 0x7d), // white (medium gray)
            Color::rgb(0x58, 0x60, 0x69), // bright black
            Color::rgb(0xcb, 0x24, 0x31), // bright red
            Color::rgb(0x28, 0xa7, 0x45), // bright green
            Color::rgb(0xdb, 0xab, 0x09), // bright yellow
            Color::rgb(0x03, 0x66, 0xd6), // bright blue (GitHub blue)
            Color::rgb(0x8a, 0x63, 0xd2), // bright magenta
            Color::rgb(0x21, 0x88, 0xff), // bright cyan
            Color::rgb(0xff, 0xff, 0xff), // bright white
        ];
        let mut palette = [Color::rgb(0, 0, 0); 256];
        for (i, c) in ansi.iter().enumerate() { palette[i] = *c; }
        for i in 16..256u16 { palette[i as usize] = Color::xterm_256(i as u8); }
        Self {
            name: "Github".into(),
            fg: Color::rgb(0x24, 0x29, 0x2e),
            bg: Color::rgb(0xf6, 0xf8, 0xfa),
            cursor: Color::rgb(0x03, 0x66, 0xd6),
            palette,
        }
    }

    /// True when this theme's background is perceptually dark (luma < 128).
    pub fn is_dark(&self) -> bool {
        let b = &self.bg;
        let luma = 299u32 * b.r as u32 + 587 * b.g as u32 + 114 * b.b as u32;
        luma < 128_000
    }

    /// All built-in presets — dark themes first, then light.
    pub fn presets() -> Vec<Theme> {
        vec![
            Theme::skyterm_blue(),
            Theme::cobalt_neon(),
            Theme::skyterm_dark(),
            Theme::skyterm_dracula(),
            Theme::skyterm_hunter(),
            Theme::skyterm_purple(),
            Theme::solarized_dark(),
            Theme::github_light(),
            Theme::solarized_light(),
            Theme::skyterm_light(),
            Theme::skyterm_atom(),
            Theme::skyterm_gray(),
            Theme::skyterm_beige(),
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
