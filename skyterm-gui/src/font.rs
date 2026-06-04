use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use freetype::face::LoadFlag;

/// JetBrains Mono Regular baked into the binary — always available.
static EMBEDDED_FONT_BYTES: &[u8] =
    include_bytes!("../resources/JetBrainsMono-Regular.ttf");

/// Sentinel path returned by [`locate_monospace_font`] to signal "use the
/// embedded font" without changing any of the PathBuf-based wiring in app.rs.
pub const EMBEDDED_FONT_PATH: &str = ":embedded:JetBrainsMono-Regular:";

/// 2D R8 glyph atlas. `locations` maps each known codepoint to its slot
/// index; the renderer derives the UV rect from `slot_x = i % glyphs_per_row`
/// and `slot_y = i / glyphs_per_row`.
pub struct Atlas {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub glyphs_per_row: u32,
    pub locations: HashMap<u32, u32>,
    /// Slot to use when a codepoint isn't in `locations`. Always populated.
    pub fallback_slot: u32,
}

const GLYPHS_PER_ROW: u32 = 32;

/// Codepoint ranges commonly used by terminal apps. ASCII first so '?' is
/// guaranteed in early slots.
pub fn default_ranges() -> Vec<RangeInclusive<u32>> {
    vec![
        0x0020..=0x007E, // ASCII printable
        0x00A0..=0x00FF, // Latin-1 supplement (°, ±, £, ·, …)
        0x0370..=0x03FF, // Greek (π for DEC graphics, math symbols)
        0x2010..=0x2027, // General punctuation (en/em dash, ellipsis, …)
        0x2190..=0x21FF, // Arrows
        0x2200..=0x22FF, // Mathematical operators (≤, ≥, ≠, …)
        0x2300..=0x23FF, // Misc technical (DEC scan lines ⎺⎻⎼⎽)
        0x2400..=0x243F, // Control pictures (␤ ␋ etc. for DEC graphics)
        0x2500..=0x257F, // Box drawing
        0x2580..=0x259F, // Block elements
        0x25A0..=0x25FF, // Geometric shapes
        0x2600..=0x26FF, // Misc symbols
        0x2700..=0x27BF, // Dingbats
    ]
}

/// Display name → file path for monospace fonts the binary knows how to find.
/// Keep alphabetized by display name for a stable Settings dropdown.
const FONT_CANDIDATES: &[(&str, &str)] = &[
    ("DejaVu Sans Mono", "/usr/share/fonts/dejavu-sans-mono-fonts/DejaVuSansMono.ttf"),
    ("DejaVu Sans Mono", "/usr/share/fonts/dejavu/DejaVuSansMono.ttf"),
    ("DejaVu Sans Mono", "/usr/share/fonts/TTF/DejaVuSansMono.ttf"),
    ("Liberation Mono", "/usr/share/fonts/liberation-mono/LiberationMono-Regular.ttf"),
    ("Liberation Mono", "/usr/share/fonts/liberation/LiberationMono-Regular.ttf"),
    ("Menlo", "/Library/Fonts/Menlo.ttc"),
    ("Menlo", "/System/Library/Fonts/Menlo.ttc"),
    ("Noto Mono", "/usr/share/fonts/google-noto-mono/NotoMono-Regular.ttf"),
    ("Noto Sans Mono", "/usr/share/fonts/google-noto/NotoSansMono-Regular.ttf"),
];

/// Every candidate font that actually exists on disk. The first existing
/// (name, path) per display name is kept — alternate paths for the same
/// distro packaging are deduped.
pub fn available_monospace_fonts() -> Vec<(String, PathBuf)> {
    let mut seen: Vec<(String, PathBuf)> = Vec::new();
    for (name, path) in FONT_CANDIDATES {
        if !Path::new(path).is_file() {
            continue;
        }
        if seen.iter().any(|(n, _)| n == name) {
            continue;
        }
        seen.push((name.to_string(), PathBuf::from(path)));
    }
    seen
}

pub fn locate_monospace_font() -> Result<PathBuf> {
    // Embedded JetBrains Mono is always the default; never fails.
    Ok(PathBuf::from(EMBEDDED_FONT_PATH))
}

/// Return the font's family name as reported by FreeType, used to make the
/// GTK chrome (right-click menu, banners) render in the same family as the
/// terminal grid.
pub fn family_name(font_path: &Path) -> Result<String> {
    let lib = freetype::Library::init().context("freetype init")?;
    let face = if font_path.to_str() == Some(EMBEDDED_FONT_PATH) {
        lib.new_memory_face(EMBEDDED_FONT_BYTES.to_vec(), 0)
            .context("loading embedded JetBrains Mono")?
    } else {
        lib.new_face(font_path, 0)
            .with_context(|| format!("opening font {}", font_path.display()))?
    };
    face.family_name()
        .ok_or_else(|| anyhow!("font reports no family name"))
}

pub fn build_atlas(font_path: &Path, size_px: u32) -> Result<Atlas> {
    build_atlas_with_ranges(font_path, size_px, &default_ranges())
}

pub fn build_atlas_with_ranges(
    font_path: &Path,
    size_px: u32,
    ranges: &[RangeInclusive<u32>],
) -> Result<Atlas> {
    let lib = freetype::Library::init().context("freetype init")?;
    let face = if font_path.to_str() == Some(EMBEDDED_FONT_PATH) {
        lib.new_memory_face(EMBEDDED_FONT_BYTES.to_vec(), 0)
            .context("loading embedded JetBrains Mono")?
    } else {
        lib.new_face(font_path, 0)
            .with_context(|| format!("opening font {}", font_path.display()))?
    };
    face.set_pixel_sizes(0, size_px)
        .context("set_pixel_sizes")?;

    let metrics = face
        .size_metrics()
        .ok_or_else(|| anyhow!("font has no size metrics"))?;
    let ascender = (metrics.ascender >> 6) as i32;
    // `max_advance` covers the widest glyph; `height` is the recommended
    // line spacing (includes the font's internal leading). Using these gives
    // each cell enough room that block elements (█▌▐▀▄) and wide box-drawing
    // glyphs fit without overflowing into neighbouring cells.
    let cell_w = ((metrics.max_advance >> 6) as u32).max(1);
    let cell_h = ((metrics.height >> 6) as u32).max(1);
    if cell_w == 0 || cell_h == 0 {
        return Err(anyhow!("font reports zero cell metrics"));
    }

    // Collect all codepoints that exist in the font (skip the ones the face
    // can't represent so we don't waste slots on tofu boxes).
    let mut codepoints: Vec<u32> = Vec::new();
    for range in ranges {
        for cp in range.clone() {
            if face.get_char_index(cp as usize).is_some() {
                codepoints.push(cp);
            }
        }
    }
    // '?' must be present as the fallback.
    if !codepoints.contains(&('?' as u32)) {
        codepoints.push('?' as u32);
    }

    let total = codepoints.len() as u32;
    let cols = GLYPHS_PER_ROW;
    let rows_in_atlas = total.div_ceil(cols);
    let width = cols * cell_w;
    let height = rows_in_atlas * cell_h;
    // 3 bytes per texel — RGB LCD sub-pixel coverage (R, G, B channels).
    let mut pixels = vec![0u8; (width as usize) * (height as usize) * 3];

    let mut locations: HashMap<u32, u32> = HashMap::with_capacity(codepoints.len());
    let mut fallback_slot: u32 = 0;

    for (slot, &cp) in codepoints.iter().enumerate() {
        let slot = slot as u32;
        if cp == '?' as u32 {
            fallback_slot = slot;
        }
        locations.insert(cp, slot);
        // TARGET_LCD: FreeType renders R/G/B sub-pixel coverage. The resulting
        // bitmap.width() is 3× the pixel column count.
        if face
            .load_char(cp as usize, LoadFlag::TARGET_LCD | LoadFlag::RENDER)
            .is_err()
        {
            continue;
        }
        let glyph = face.glyph();
        let bitmap = glyph.bitmap();
        let bm_byte_w = bitmap.width(); // bytes per row = 3 × pixel columns
        let bm_px_w = bm_byte_w / 3;   // actual pixel columns
        let bm_h = bitmap.rows();
        let pitch = bitmap.pitch().abs() as usize;
        let buf = bitmap.buffer();
        let bm_left = glyph.bitmap_left();
        let bm_top = glyph.bitmap_top();

        let slot_x = slot % cols;
        let slot_y = slot / cols;
        let cell_left = (slot_x * cell_w) as i32;
        let cell_top = (slot_y * cell_h) as i32;
        let cell_right = cell_left + cell_w as i32;
        let cell_bottom = cell_top + cell_h as i32;
        let cell_origin_x = cell_left + bm_left;
        let cell_origin_y = cell_top + (ascender - bm_top);

        for y in 0..bm_h {
            let dst_y = cell_origin_y + y;
            if dst_y < cell_top || dst_y >= cell_bottom {
                continue;
            }
            for x_px in 0..bm_px_w {
                let dst_x = cell_origin_x + x_px;
                if dst_x < cell_left || dst_x >= cell_right {
                    continue;
                }
                let src = (y as usize) * pitch + (x_px as usize) * 3;
                let dst = ((dst_y as usize) * (width as usize) + (dst_x as usize)) * 3;
                pixels[dst]     = buf[src];
                pixels[dst + 1] = buf[src + 1];
                pixels[dst + 2] = buf[src + 2];
            }
        }
    }

    Ok(Atlas {
        pixels,
        width,
        height,
        cell_w,
        cell_h,
        glyphs_per_row: cols,
        locations,
        fallback_slot,
    })
}
