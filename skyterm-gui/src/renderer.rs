use std::collections::HashMap;

use anyhow::{anyhow, Result};
use glow::HasContext;
use skyterm_core::{
    theme::{Color, Theme},
    CursorShape, Grid,
};

use crate::font::Atlas;

/// A drag-to-select range over visible (view-relative) cell coordinates.
/// Either endpoint may be the lexically-smaller one; `contains` normalizes.
#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub anchor: (usize, usize),
    pub active: (usize, usize),
}

impl Selection {
    /// True when `(row, col)` falls inside the linear (reading-order) span
    /// between the two endpoints, inclusive on both ends.
    pub fn contains(&self, row: usize, col: usize) -> bool {
        let (a, b) = if self.anchor <= self.active {
            (self.anchor, self.active)
        } else {
            (self.active, self.anchor)
        };
        let (sr, sc) = a;
        let (er, ec) = b;
        if row < sr || row > er {
            return false;
        }
        if sr == er {
            col >= sc && col <= ec
        } else if row == sr {
            col >= sc
        } else if row == er {
            col <= ec
        } else {
            true
        }
    }

    /// Normalized `(start, end)` where `start <= end` in reading order. Use
    /// when extracting text for the clipboard.
    pub fn ordered(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.active {
            (self.anchor, self.active)
        } else {
            (self.active, self.anchor)
        }
    }
}

const SELECTION_BG: Color = Color::rgb(0x3a, 0x5e, 0x8e);

const BG_VERT: &str = r#"#version 330 core
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec3 a_color;
out vec3 v_color;
void main() {
    gl_Position = vec4(a_pos, 0.0, 1.0);
    v_color = a_color;
}
"#;

const BG_FRAG: &str = r#"#version 330 core
in vec3 v_color;
out vec4 frag;
void main() {
    frag = vec4(v_color, 1.0);
}
"#;

const FG_VERT: &str = r#"#version 330 core
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec3 a_fg;
layout(location = 3) in vec3 a_bg;
out vec2 v_uv;
out vec3 v_fg;
out vec3 v_bg;
void main() {
    gl_Position = vec4(a_pos, 0.0, 1.0);
    v_uv = a_uv;
    v_fg = a_fg;
    v_bg = a_bg;
}
"#;

const FG_FRAG: &str = r#"#version 330 core
in vec2 v_uv;
in vec3 v_fg;
in vec3 v_bg;
out vec4 frag;
uniform sampler2D u_atlas;
void main() {
    vec3 lcd = texture(u_atlas, v_uv).rgb;
    frag = vec4(v_fg * lcd + v_bg * (1.0 - lcd), 1.0);
}
"#;

pub struct Renderer {
    gl: glow::Context,
    bg_program: glow::Program,
    bg_vao: glow::VertexArray,
    bg_vbo: glow::Buffer,
    fg_program: glow::Program,
    fg_vao: glow::VertexArray,
    fg_vbo: glow::Buffer,
    atlas_tex: glow::Texture,
    cell_w: u32,
    cell_h: u32,
    atlas_w: u32,
    atlas_h: u32,
    glyphs_per_row: u32,
    glyph_locations: HashMap<u32, u32>,
    fallback_slot: u32,
    viewport_w: i32,
    viewport_h: i32,
    clear_color: Color,
    bg_scratch: Vec<f32>,
    fg_scratch: Vec<f32>,
}

impl Renderer {
    pub fn new(gl: glow::Context, atlas: &Atlas) -> Result<Self> {
        unsafe {
            let bg_program = link_program(&gl, BG_VERT, BG_FRAG)?;
            let fg_program = link_program(&gl, FG_VERT, FG_FRAG)?;

            let (bg_vao, bg_vbo) = create_bg_vao(&gl)?;
            let (fg_vao, fg_vbo) = create_fg_vao(&gl)?;

            let atlas_tex = gl
                .create_texture()
                .map_err(|e| anyhow!("create_texture: {e}"))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(atlas_tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGB8 as i32,
                atlas.width as i32,
                atlas.height as i32,
                0,
                glow::RGB,
                glow::UNSIGNED_BYTE,
                Some(&atlas.pixels),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );

            Ok(Self {
                gl,
                bg_program,
                bg_vao,
                bg_vbo,
                fg_program,
                fg_vao,
                fg_vbo,
                atlas_tex,
                cell_w: atlas.cell_w,
                cell_h: atlas.cell_h,
                atlas_w: atlas.width,
                atlas_h: atlas.height,
                glyphs_per_row: atlas.glyphs_per_row,
                glyph_locations: atlas.locations.clone(),
                fallback_slot: atlas.fallback_slot,
                viewport_w: 0,
                viewport_h: 0,
                clear_color: Color::DEFAULT_BG,
                bg_scratch: Vec::new(),
                fg_scratch: Vec::new(),
            })
        }
    }

    pub fn resize(&mut self, w: i32, h: i32) {
        self.viewport_w = w;
        self.viewport_h = h;
    }

    pub fn viewport(&self) -> (i32, i32) {
        (self.viewport_w, self.viewport_h)
    }

    /// Replace the glyph atlas in-place. Re-uploads the texture and refreshes
    /// cell metrics + glyph lookup state. The caller must ensure the GLArea's
    /// context is current before invoking this (e.g. via `make_current`).
    pub fn set_atlas(&mut self, atlas: &Atlas) {
        unsafe {
            let gl = &self.gl;
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas_tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGB8 as i32,
                atlas.width as i32,
                atlas.height as i32,
                0,
                glow::RGB,
                glow::UNSIGNED_BYTE,
                Some(&atlas.pixels),
            );
        }
        self.cell_w = atlas.cell_w;
        self.cell_h = atlas.cell_h;
        self.atlas_w = atlas.width;
        self.atlas_h = atlas.height;
        self.glyphs_per_row = atlas.glyphs_per_row;
        self.glyph_locations = atlas.locations.clone();
        self.fallback_slot = atlas.fallback_slot;
    }

    pub fn render(&mut self, grid: &Grid, selection: Option<Selection>, theme: &Theme, cursor_on: bool) {
        if self.viewport_w <= 0 || self.viewport_h <= 0 {
            return;
        }

        self.bg_scratch.clear();
        self.fg_scratch.clear();
        self.clear_color = theme.bg;

        let cw = self.cell_w as f32;
        let ch = self.cell_h as f32;
        let vw = self.viewport_w as f32;
        let vh = self.viewport_h as f32;
        let atlas_w = self.atlas_w as f32;
        let atlas_h = self.atlas_h as f32;
        let glyphs_per_row = self.glyphs_per_row;
        let to_ndc_x = |x: f32| (x / vw) * 2.0 - 1.0;
        let to_ndc_y = |y: f32| 1.0 - (y / vh) * 2.0;

        // Pass 1: per-cell background quads. Selection wins over both the
        // cell's own bg and the default bg — that's how the user sees their
        // drag highlight even on otherwise-blank space.
        for row in 0..grid.rows() {
            for col in 0..grid.cols() {
                let selected = selection.map(|s| s.contains(row, col)).unwrap_or(false);
                let cell = grid.visible_cell(row, col);
                let draw = if selected {
                    Some(SELECTION_BG)
                } else if matches!(cell.bg, skyterm_core::theme::CellColor::Default) {
                    None
                } else {
                    Some(theme.resolve_bg(cell.bg))
                };
                let Some(bg) = draw else {
                    continue;
                };
                let (px0, py0) = (col as f32 * cw, row as f32 * ch);
                push_solid_quad(
                    &mut self.bg_scratch,
                    to_ndc_x(px0),
                    to_ndc_y(py0),
                    to_ndc_x(px0 + cw),
                    to_ndc_y(py0 + ch),
                    bg,
                );
            }
        }

        // Cursor background quad. Shape is set by DECSCUSR from the app;
        // defaults to Block. Hidden during off-blink phase, scrollback, or ?25l.
        let cursor_cell = if cursor_on && grid.cursor_visible() {
            grid.visible_cursor().filter(|&(r, c)| r < grid.rows() && c < grid.cols())
        } else {
            None
        };
        let cursor_shape = grid.cursor_shape();

        if let Some((cur_row, cur_col)) = cursor_cell {
            let px0 = cur_col as f32 * cw;
            let py0 = cur_row as f32 * ch;
            match cursor_shape {
                CursorShape::Block => {
                    // Full cell — glyph pass will invert the character on top.
                    push_solid_quad(
                        &mut self.bg_scratch,
                        to_ndc_x(px0), to_ndc_y(py0),
                        to_ndc_x(px0 + cw), to_ndc_y(py0 + ch),
                        theme.cursor,
                    );
                }
                CursorShape::Underline => {
                    let h = (ch * 0.12).max(2.0);
                    push_solid_quad(
                        &mut self.bg_scratch,
                        to_ndc_x(px0), to_ndc_y(py0 + ch - h),
                        to_ndc_x(px0 + cw), to_ndc_y(py0 + ch),
                        theme.cursor,
                    );
                }
                CursorShape::Bar => {
                    let w = (cw * 0.12).max(2.0);
                    push_solid_quad(
                        &mut self.bg_scratch,
                        to_ndc_x(px0), to_ndc_y(py0),
                        to_ndc_x(px0 + w), to_ndc_y(py0 + ch),
                        theme.cursor,
                    );
                }
            }
        }

        // Pass 2: glyph quads. LCD sub-pixel blending needs the bg color per
        // cell so the shader can mix fg and bg per R/G/B channel.
        // At the block-cursor cell we invert fg/bg so the character reads
        // against the cursor background color.
        for row in 0..grid.rows() {
            for col in 0..grid.cols() {
                let cell = grid.visible_cell(row, col);
                if cell.ch == ' ' {
                    continue;
                }
                let selected = selection.map(|s| s.contains(row, col)).unwrap_or(false);
                let at_block_cursor = matches!(cursor_shape, CursorShape::Block)
                    && cursor_cell == Some((row, col));
                let (fg_color, bg_color) = if at_block_cursor {
                    // Character rendered in terminal-bg over cursor-color block.
                    (theme.bg, theme.cursor)
                } else if selected {
                    (theme.resolve_fg(cell.fg), SELECTION_BG)
                } else {
                    (theme.resolve_fg(cell.fg), theme.resolve_bg(cell.bg))
                };
                let cp = cell.ch as u32;
                let slot = *self.glyph_locations.get(&cp).unwrap_or(&self.fallback_slot);
                let slot_x = slot % glyphs_per_row;
                let slot_y = slot / glyphs_per_row;
                let (px0, py0) = (col as f32 * cw, row as f32 * ch);
                let u0 = (slot_x as f32 * cw) / atlas_w;
                let u1 = u0 + cw / atlas_w;
                let v0 = (slot_y as f32 * ch) / atlas_h;
                let v1 = v0 + ch / atlas_h;
                push_glyph_quad(
                    &mut self.fg_scratch,
                    to_ndc_x(px0),
                    to_ndc_y(py0),
                    to_ndc_x(px0 + cw),
                    to_ndc_y(py0 + ch),
                    u0,
                    v0,
                    u1,
                    v1,
                    fg_color,
                    bg_color,
                );
            }
        }

        unsafe {
            let gl = &self.gl;
            gl.viewport(0, 0, self.viewport_w, self.viewport_h);
            let bg = self.clear_color;
            gl.clear_color(
                bg.r as f32 / 255.0,
                bg.g as f32 / 255.0,
                bg.b as f32 / 255.0,
                1.0,
            );
            gl.clear(glow::COLOR_BUFFER_BIT);

            // Bg pass — opaque.
            gl.disable(glow::BLEND);
            if !self.bg_scratch.is_empty() {
                gl.use_program(Some(self.bg_program));
                gl.bind_vertex_array(Some(self.bg_vao));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.bg_vbo));
                let bytes: &[u8] = bytemuck::cast_slice(&self.bg_scratch);
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
                // 5 floats per vertex (x, y, r, g, b).
                let n = (self.bg_scratch.len() / 5) as i32;
                gl.draw_arrays(glow::TRIANGLES, 0, n);
            }

            // Fg pass — LCD sub-pixel blending done in the shader; output is
            // fully opaque so no GPU blending needed.
            gl.disable(glow::BLEND);
            if !self.fg_scratch.is_empty() {
                gl.use_program(Some(self.fg_program));
                gl.bind_vertex_array(Some(self.fg_vao));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.fg_vbo));
                let bytes: &[u8] = bytemuck::cast_slice(&self.fg_scratch);
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas_tex));
                // 10 floats per vertex (x, y, u, v, fg.r, fg.g, fg.b, bg.r, bg.g, bg.b).
                let n = (self.fg_scratch.len() / 10) as i32;
                gl.draw_arrays(glow::TRIANGLES, 0, n);
            }
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.bg_program);
            self.gl.delete_program(self.fg_program);
            self.gl.delete_vertex_array(self.bg_vao);
            self.gl.delete_vertex_array(self.fg_vao);
            self.gl.delete_buffer(self.bg_vbo);
            self.gl.delete_buffer(self.fg_vbo);
            self.gl.delete_texture(self.atlas_tex);
        }
    }
}

unsafe fn create_bg_vao(gl: &glow::Context) -> Result<(glow::VertexArray, glow::Buffer)> {
    let vao = gl
        .create_vertex_array()
        .map_err(|e| anyhow!("create_vertex_array: {e}"))?;
    gl.bind_vertex_array(Some(vao));
    let vbo = gl.create_buffer().map_err(|e| anyhow!("create_buffer: {e}"))?;
    gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
    let stride = (5 * std::mem::size_of::<f32>()) as i32;
    gl.enable_vertex_attrib_array(0);
    gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
    gl.enable_vertex_attrib_array(1);
    gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, stride, (2 * 4) as i32);
    Ok((vao, vbo))
}

unsafe fn create_fg_vao(gl: &glow::Context) -> Result<(glow::VertexArray, glow::Buffer)> {
    let vao = gl
        .create_vertex_array()
        .map_err(|e| anyhow!("create_vertex_array: {e}"))?;
    gl.bind_vertex_array(Some(vao));
    let vbo = gl.create_buffer().map_err(|e| anyhow!("create_buffer: {e}"))?;
    gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
    // 10 floats per vertex: pos(2) + uv(2) + fg(3) + bg(3)
    let stride = (10 * std::mem::size_of::<f32>()) as i32;
    gl.enable_vertex_attrib_array(0);
    gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
    gl.enable_vertex_attrib_array(1);
    gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, (2 * 4) as i32);
    gl.enable_vertex_attrib_array(2);
    gl.vertex_attrib_pointer_f32(2, 3, glow::FLOAT, false, stride, (4 * 4) as i32);
    gl.enable_vertex_attrib_array(3);
    gl.vertex_attrib_pointer_f32(3, 3, glow::FLOAT, false, stride, (7 * 4) as i32);
    Ok((vao, vbo))
}

fn push_solid_quad(buf: &mut Vec<f32>, x0: f32, y0: f32, x1: f32, y1: f32, c: Color) {
    let r = c.r as f32 / 255.0;
    let g = c.g as f32 / 255.0;
    let b = c.b as f32 / 255.0;
    let v = [
        x0, y0, r, g, b,
        x1, y0, r, g, b,
        x1, y1, r, g, b,
        x0, y0, r, g, b,
        x1, y1, r, g, b,
        x0, y1, r, g, b,
    ];
    buf.extend_from_slice(&v);
}

#[allow(clippy::too_many_arguments)]
fn push_glyph_quad(
    buf: &mut Vec<f32>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
    fg: Color,
    bg: Color,
) {
    let fr = fg.r as f32 / 255.0;
    let fg_g = fg.g as f32 / 255.0;
    let fb = fg.b as f32 / 255.0;
    let br = bg.r as f32 / 255.0;
    let bg_g = bg.g as f32 / 255.0;
    let bb = bg.b as f32 / 255.0;
    let v = [
        x0, y0, u0, v0, fr, fg_g, fb, br, bg_g, bb,
        x1, y0, u1, v0, fr, fg_g, fb, br, bg_g, bb,
        x1, y1, u1, v1, fr, fg_g, fb, br, bg_g, bb,
        x0, y0, u0, v0, fr, fg_g, fb, br, bg_g, bb,
        x1, y1, u1, v1, fr, fg_g, fb, br, bg_g, bb,
        x0, y1, u0, v1, fr, fg_g, fb, br, bg_g, bb,
    ];
    buf.extend_from_slice(&v);
}

unsafe fn link_program(gl: &glow::Context, vs_src: &str, fs_src: &str) -> Result<glow::Program> {
    let vs = compile_shader(gl, glow::VERTEX_SHADER, vs_src)?;
    let fs = compile_shader(gl, glow::FRAGMENT_SHADER, fs_src)?;
    let program = gl
        .create_program()
        .map_err(|e| anyhow!("create_program: {e}"))?;
    gl.attach_shader(program, vs);
    gl.attach_shader(program, fs);
    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        let log = gl.get_program_info_log(program);
        return Err(anyhow!("program link failed: {log}"));
    }
    gl.detach_shader(program, vs);
    gl.detach_shader(program, fs);
    gl.delete_shader(vs);
    gl.delete_shader(fs);
    Ok(program)
}

unsafe fn compile_shader(gl: &glow::Context, kind: u32, src: &str) -> Result<glow::Shader> {
    let shader = gl
        .create_shader(kind)
        .map_err(|e| anyhow!("create_shader: {e}"))?;
    gl.shader_source(shader, src);
    gl.compile_shader(shader);
    if !gl.get_shader_compile_status(shader) {
        let log = gl.get_shader_info_log(shader);
        return Err(anyhow!("shader compile failed: {log}"));
    }
    Ok(shader)
}
