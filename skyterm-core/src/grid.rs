use std::collections::VecDeque;
use std::ops::{Deref, DerefMut, Index, IndexMut};

use crate::theme::CellColor;

const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

/// Which mouse-reporting protocol the foreground app has enabled.
///
/// Scroll events are forwarded to the PTY when any mode other than `None` is
/// active, encoded as button 64 (up) / 65 (down).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MouseMode {
    /// No mouse reporting — scroll wheel drives scrollback.
    #[default]
    None,
    /// ?1000 — report button press/release only.
    Normal,
    /// ?1002 — report press/release + motion while a button is held.
    ButtonMotion,
    /// ?1003 — report all motion events.
    AnyMotion,
}

/// Terminal cursor shape, set by DECSCUSR (`CSI Ps SP q`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CursorShape {
    /// Filled block covering the full cell (modes 0, 1, 2). Default.
    #[default]
    Block,
    /// Thin horizontal bar at the bottom of the cell (modes 3, 4).
    Underline,
    /// Thin vertical bar at the left edge of the cell (modes 5, 6).
    Bar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: CellColor,
    pub bg: CellColor,
}

impl Cell {
    pub const EMPTY: Cell = Cell {
        ch: ' ',
        fg: CellColor::Default,
        bg: CellColor::Default,
    };
}

impl Default for Cell {
    fn default() -> Self {
        Cell::EMPTY
    }
}

/// One row of cells plus a `wrapped` flag recording *why* the line ended.
///
/// `wrapped == true` means the text ran off the right edge and auto-wrapped
/// onto the next row — a "soft" break with no real newline in the byte stream.
/// `false` means the row was terminated by an explicit newline (or never
/// filled). On resize we rejoin soft-wrapped runs into logical lines and
/// re-split them at the new width, so already-printed output reflows the way
/// VTE-based terminals (gnome-terminal, Terminator) do.
///
/// `Deref`/`Index` to the inner `Vec<Cell>` keep every existing `row[col]` and
/// `row.len()` call site working unchanged; the flag rides along automatically
/// through `remove`/`insert`/`push_back`, so it can't desync from its cells.
#[derive(Clone)]
pub(crate) struct Row {
    cells: Vec<Cell>,
    wrapped: bool,
}

impl Row {
    fn blank(len: usize, cell: Cell) -> Row {
        Row {
            cells: vec![cell; len],
            wrapped: false,
        }
    }
}

impl Deref for Row {
    type Target = Vec<Cell>;
    fn deref(&self) -> &Vec<Cell> {
        &self.cells
    }
}

impl DerefMut for Row {
    fn deref_mut(&mut self) -> &mut Vec<Cell> {
        &mut self.cells
    }
}

impl Index<usize> for Row {
    type Output = Cell;
    fn index(&self, i: usize) -> &Cell {
        &self.cells[i]
    }
}

impl IndexMut<usize> for Row {
    fn index_mut(&mut self, i: usize) -> &mut Cell {
        &mut self.cells[i]
    }
}

/// Fixed-size character grid with a wrapping cursor and current-SGR state.
///
/// M2 scope so far: cursor placement, erase, insert/delete chars, scroll
/// (no per-region scroll yet — full-screen), per-cell fg/bg, bold tracked but
/// not yet rendered. No scrollback.
pub struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<Row>,
    cursor_row: usize,
    cursor_col: usize,
    current_fg: CellColor,
    current_bg: CellColor,
    bold: bool,
    reverse: bool,
    cursor_visible: bool,
    cursor_shape: CursorShape,
    saved: Option<SavedCursor>,
    /// When `Some`, the live screen is the alt screen and the saved main
    /// screen state lives here, waiting to be restored on exit.
    alt: Option<AltScreenSave>,
    scrollback: VecDeque<Row>,
    scrollback_max: usize,
    /// 0 = viewing live screen at bottom. N>0 = scrolled up N lines into
    /// scrollback. Clamped to scrollback.len() on set.
    view_offset: usize,
    /// DECSTBM scroll region, 0-based inclusive. Defaults to the full screen.
    /// Linefeed at the bottom margin scrolls only [scroll_top, scroll_bottom];
    /// rows outside that band stay put. Scrollback is fed only when the region
    /// covers the entire screen — partial-region scrolls (e.g. htop's process
    /// list under a pinned header) are transient.
    scroll_top: usize,
    scroll_bottom: usize,
    /// Which mouse-reporting mode is active (set by ?1000/1002/1003 h/l).
    pub mouse_mode: MouseMode,
    /// Whether the SGR mouse extension (?1006) is active. When true, scroll
    /// events are encoded as `CSI < 64 ; col ; row M` instead of raw bytes.
    pub mouse_sgr: bool,
    /// DECCKM (?1). When set, the cursor/arrow keys transmit in *application*
    /// mode (`ESC O A` …) instead of normal mode (`ESC [ A` …). ncurses apps
    /// (htop, less, vim) enable this via `keypad()` and won't recognise arrows
    /// sent in the wrong mode.
    pub app_cursor_keys: bool,
    dirty: u64,
}

#[derive(Clone)]
struct SavedCursor {
    row: usize,
    col: usize,
    fg: CellColor,
    bg: CellColor,
    bold: bool,
    reverse: bool,
}

struct AltScreenSave {
    cells: Vec<Row>,
    cursor_row: usize,
    cursor_col: usize,
    fg: CellColor,
    bg: CellColor,
    bold: bool,
    reverse: bool,
    /// The main screen's `saved` slot, held while we're in alt so the alt
    /// screen has its own independent DECSC/DECRC slot.
    main_saved: Option<SavedCursor>,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cells = (0..rows).map(|_| Row::blank(cols, Cell::EMPTY)).collect();
        Self {
            cols,
            rows,
            cells,
            cursor_row: 0,
            cursor_col: 0,
            current_fg: CellColor::Default,
            current_bg: CellColor::Default,
            bold: false,
            reverse: false,
            cursor_visible: true,
            cursor_shape: CursorShape::default(),
            saved: None,
            alt: None,
            scrollback: VecDeque::new(),
            scrollback_max: DEFAULT_SCROLLBACK_LINES,
            view_offset: 0,
            mouse_mode: MouseMode::default(),
            mouse_sgr: false,
            app_cursor_keys: false,
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            dirty: 1,
        }
    }

    /// DECSTBM. 1-based row indices; 0 means "default" (top=1, bottom=rows).
    /// Sequences that name a degenerate region (top >= bottom, out of bounds)
    /// are ignored, matching xterm. Per spec the cursor moves to home.
    pub fn set_scroll_region(&mut self, top_1based: usize, bottom_1based: usize) {
        let top = if top_1based == 0 { 0 } else { top_1based - 1 };
        let bottom = if bottom_1based == 0 {
            self.rows.saturating_sub(1)
        } else {
            bottom_1based - 1
        };
        if bottom >= self.rows || top >= bottom {
            return;
        }
        self.scroll_top = top;
        self.scroll_bottom = bottom;
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.bump_dirty();
    }

    pub fn scroll_region(&self) -> (usize, usize) {
        (self.scroll_top, self.scroll_bottom)
    }

    /// True when the scroll region covers every row — the only configuration
    /// under which lines scrolled off the top should reach the user's history.
    fn region_is_full(&self) -> bool {
        self.scroll_top == 0 && self.scroll_bottom + 1 == self.rows
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn cursor_shape(&self) -> CursorShape {
        self.cursor_shape
    }

    pub fn set_cursor_shape(&mut self, shape: CursorShape) {
        self.cursor_shape = shape;
    }

    pub fn set_cursor_visible(&mut self, v: bool) {
        if self.cursor_visible != v {
            self.cursor_visible = v;
            self.bump_dirty();
        }
    }

    pub fn is_alt_screen(&self) -> bool {
        self.alt.is_some()
    }

    pub fn save_cursor(&mut self) {
        self.saved = Some(SavedCursor {
            row: self.cursor_row,
            col: self.cursor_col,
            fg: self.current_fg,
            bg: self.current_bg,
            bold: self.bold,
            reverse: self.reverse,
        });
    }

    pub fn restore_cursor(&mut self) {
        if let Some(s) = self.saved.clone() {
            let rmax = self.rows.saturating_sub(1);
            let cmax = self.cols.saturating_sub(1);
            self.cursor_row = s.row.min(rmax);
            self.cursor_col = s.col.min(cmax);
            self.current_fg = s.fg;
            self.current_bg = s.bg;
            self.bold = s.bold;
            self.reverse = s.reverse;
            self.bump_dirty();
        }
    }

    pub fn enter_alt_screen(&mut self) {
        if self.alt.is_some() {
            return;
        }
        let fresh = (0..self.rows)
            .map(|_| Row::blank(self.cols, Cell::EMPTY))
            .collect();
        let saved_cells = std::mem::replace(&mut self.cells, fresh);
        self.alt = Some(AltScreenSave {
            cells: saved_cells,
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            fg: self.current_fg,
            bg: self.current_bg,
            bold: self.bold,
            reverse: self.reverse,
            main_saved: self.saved.take(),
        });
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.current_fg = CellColor::Default;
        self.current_bg = CellColor::Default;
        self.bold = false;
        self.reverse = false;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows.saturating_sub(1);
        // Snap view back to live screen — scrollback belongs to main.
        self.view_offset = 0;
        self.bump_dirty();
    }

    pub fn exit_alt_screen(&mut self) {
        if let Some(save) = self.alt.take() {
            // Restoring across a resize: if the saved main cells don't match
            // current dimensions, reshape them with blank padding/truncation.
            let mut cells = save.cells;
            if cells.len() != self.rows {
                cells.resize_with(self.rows, || Row::blank(self.cols, Cell::EMPTY));
            }
            for row in &mut cells {
                row.resize(self.cols, Cell::EMPTY);
            }
            self.cells = cells;
            let rmax = self.rows.saturating_sub(1);
            let cmax = self.cols.saturating_sub(1);
            self.cursor_row = save.cursor_row.min(rmax);
            self.cursor_col = save.cursor_col.min(cmax);
            self.current_fg = save.fg;
            self.current_bg = save.bg;
            self.bold = save.bold;
            self.reverse = save.reverse;
            self.saved = save.main_saved;
            self.scroll_top = 0;
            self.scroll_bottom = self.rows.saturating_sub(1);
            self.view_offset = 0;
            self.bump_dirty();
        }
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    pub fn view_offset(&self) -> usize {
        self.view_offset
    }

    /// Set how many lines above the live bottom the user is viewing. Clamped.
    pub fn set_view_offset(&mut self, offset: usize) {
        let max = self.scrollback.len();
        let new_offset = offset.min(max);
        if new_offset != self.view_offset {
            self.view_offset = new_offset;
            self.bump_dirty();
        }
    }

    pub fn scrollback_max(&self) -> usize {
        self.scrollback_max
    }

    /// `n == 0` is treated as "infinite" — no trimming, lines accumulate
    /// forever. Any positive value is the upper bound, with the oldest
    /// lines dropped first.
    pub fn set_scrollback_max(&mut self, n: usize) {
        self.scrollback_max = n;
        if n == 0 {
            return;
        }
        while self.scrollback.len() > n {
            self.scrollback.pop_front();
            if self.view_offset > self.scrollback.len() {
                self.view_offset = self.scrollback.len();
            }
        }
    }

    /// Total addressable logical rows: scrollback history plus the live screen.
    /// Logical row 0 is the oldest scrollback line; the last `rows` indices are
    /// the current screen, top→bottom. Unlike view rows, logical rows are stable
    /// under scrolling, so a selection that spans more than one screenful stores
    /// logical coordinates and survives auto-scroll.
    pub fn logical_rows(&self) -> usize {
        self.scrollback.len() + self.rows
    }

    /// Convert a view-relative row (0 = topmost visible) to its logical row.
    pub fn view_to_logical(&self, view_row: usize) -> usize {
        self.scrollback.len().saturating_sub(self.view_offset) + view_row
    }

    /// Cell at an absolute logical row (see [`logical_rows`](Self::logical_rows)).
    pub fn logical_cell(&self, logical: usize, col: usize) -> Cell {
        let sb_len = self.scrollback.len();
        if logical < sb_len {
            self.scrollback[logical]
                .get(col)
                .copied()
                .unwrap_or(Cell::EMPTY)
        } else {
            let r = logical - sb_len;
            if r < self.rows && col < self.cols {
                self.cells[r][col]
            } else {
                Cell::EMPTY
            }
        }
    }

    /// Whether an absolute logical row ends in a soft (auto) wrap — i.e. its
    /// text continues on the next row.
    pub fn logical_row_wrapped(&self, logical: usize) -> bool {
        let sb_len = self.scrollback.len();
        if logical < sb_len {
            self.scrollback[logical].wrapped
        } else {
            let r = logical - sb_len;
            r < self.rows && self.cells[r].wrapped
        }
    }

    /// Cell at view-relative position (0 = topmost visible row).
    /// When scrolled up, the top of the view shows old scrollback lines.
    pub fn visible_cell(&self, view_row: usize, col: usize) -> Cell {
        self.logical_cell(self.view_to_logical(view_row), col)
    }

    /// Whether the view row at `view_row` ends in a soft (auto) wrap — i.e. its
    /// text continues on the next row. Used to select / copy a whole logical
    /// line across its wrapped continuation rows.
    pub fn visible_row_wrapped(&self, view_row: usize) -> bool {
        self.logical_row_wrapped(self.view_to_logical(view_row))
    }

    /// View-relative cursor position. None if cursor is scrolled out of view.
    pub fn visible_cursor(&self) -> Option<(usize, usize)> {
        let view_row = self.cursor_row + self.view_offset;
        if view_row < self.rows {
            Some((view_row, self.cursor_col))
        } else {
            None
        }
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    pub fn cell(&self, row: usize, col: usize) -> Cell {
        self.cells[row][col]
    }

    pub fn row(&self, row: usize) -> &[Cell] {
        &self.cells[row].cells
    }

    pub fn dirty_counter(&self) -> u64 {
        self.dirty
    }

    pub fn current_fg(&self) -> CellColor {
        self.current_fg
    }

    pub fn current_bg(&self) -> CellColor {
        self.current_bg
    }

    pub fn set_fg(&mut self, c: CellColor) {
        self.current_fg = c;
    }

    pub fn set_bg(&mut self, c: CellColor) {
        self.current_bg = c;
    }

    pub fn set_bold(&mut self, b: bool) {
        self.bold = b;
    }

    pub fn set_reverse(&mut self, v: bool) {
        self.reverse = v;
    }

    pub fn reset_attrs(&mut self) {
        self.current_fg = CellColor::Default;
        self.current_bg = CellColor::Default;
        self.bold = false;
        self.reverse = false;
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        if cols == 0 || rows == 0 {
            return;
        }
        // Alt screen (vim, htop, …) is a full-screen TUI that repaints itself
        // on SIGWINCH, so reflowing it is pointless and would fight the app.
        // Just pad/truncate it (and the saved main buffer underneath). The main
        // scrollback is left untouched; it reflows when we're back on it.
        if self.alt.is_some() {
            self.resize_simple(cols, rows);
            return;
        }
        self.reflow(cols, rows);
    }

    /// Pad/truncate every row to the new geometry without rejoining wrapped
    /// lines. Used for the alt screen, where reflow is undesirable.
    fn resize_simple(&mut self, cols: usize, rows: usize) {
        let blank = self.blank_cell();
        self.cells.resize_with(rows, || Row::blank(cols, blank));
        for line in &mut self.cells {
            line.resize(cols, blank);
        }
        if let Some(alt) = self.alt.as_mut() {
            alt.cells.resize_with(rows, || Row::blank(cols, Cell::EMPTY));
            for line in &mut alt.cells {
                line.resize(cols, Cell::EMPTY);
            }
        }
        self.cols = cols;
        self.rows = rows;
        self.cursor_row = self.cursor_row.min(rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(cols.saturating_sub(1));
        self.scroll_top = 0;
        self.scroll_bottom = rows.saturating_sub(1);
        self.bump_dirty();
    }

    /// Rejoin soft-wrapped runs into logical lines, then re-split them at the
    /// new column count — the reflow that VTE-based terminals do. The cursor's
    /// absolute position in the text is preserved, and the view snaps to the
    /// live bottom.
    fn reflow(&mut self, cols: usize, rows: usize) {
        // 1. Flatten scrollback + the in-use part of the screen into one stream
        //    of (row, is_cursor_row) entries. Rows below the cursor / last
        //    non-blank row are trailing padding we don't want to preserve as
        //    content — they'd otherwise pin the prompt to the top after a grow.
        let last_nonblank = self
            .cells
            .iter()
            .rposition(|r| r.iter().any(|c| *c != Cell::EMPTY));
        let content_rows = last_nonblank
            .map(|i| i + 1)
            .unwrap_or(0)
            .max(self.cursor_row + 1);

        // Cursor's absolute cell offset, tracked as (logical line, column).
        let cursor_stream_row = self.scrollback.len() + self.cursor_row;

        // 2. Build logical lines. A run of rows joined by `wrapped` flags is one
        //    logical line; we concatenate their cells (full width for wrapped
        //    segments, trailing blanks trimmed only on the terminating row).
        struct Logical {
            cells: Vec<Cell>,
            // Cursor offset within this logical line, if the cursor sits here.
            cursor_col: Option<usize>,
        }
        let mut logicals: Vec<Logical> = Vec::new();
        let mut cur: Vec<Cell> = Vec::new();
        let mut cur_cursor: Option<usize> = None;

        let stream = self
            .scrollback
            .iter()
            .chain(self.cells.iter().take(content_rows));

        for (stream_row, row) in stream.enumerate() {
            if stream_row == cursor_stream_row {
                cur_cursor = Some(cur.len() + self.cursor_col.min(row.len()));
            }
            if row.wrapped {
                // Soft wrap: the whole row is content; the line continues.
                cur.extend_from_slice(&row.cells);
            } else {
                // Hard break: trim trailing blanks, emit the logical line.
                let mut end = row.cells.len();
                while end > 0 && row.cells[end - 1] == Cell::EMPTY {
                    end -= 1;
                }
                cur.extend_from_slice(&row.cells[..end]);
                logicals.push(Logical {
                    cells: std::mem::take(&mut cur),
                    cursor_col: cur_cursor.take(),
                });
            }
        }
        // The cursor row may be a `wrapped` row whose run hasn't been closed by
        // a hard break (mid-wrap cursor); flush whatever's pending.
        if !cur.is_empty() || cur_cursor.is_some() {
            logicals.push(Logical {
                cells: cur,
                cursor_col: cur_cursor.take(),
            });
        }

        // 3. Re-split each logical line into rows of `cols`. Every chunk but the
        //    last gets wrapped=true. Empty logical lines stay one blank row.
        let blank = Cell::EMPTY;
        let mut out: Vec<Row> = Vec::new();
        let mut new_cursor_abs: Option<(usize, usize)> = None; // (row index, col)
        for lg in logicals {
            let first_out_row = out.len();
            if lg.cells.is_empty() {
                if lg.cursor_col.is_some() {
                    new_cursor_abs = Some((out.len(), 0));
                }
                out.push(Row::blank(cols, blank));
                continue;
            }
            let mut chunks = lg.cells.chunks(cols).peekable();
            while let Some(chunk) = chunks.next() {
                let mut cells = chunk.to_vec();
                cells.resize(cols, blank);
                let wrapped = chunks.peek().is_some();
                out.push(Row {
                    cells,
                    wrapped,
                });
            }
            if let Some(off) = lg.cursor_col {
                let r = first_out_row + off / cols;
                let c = (off % cols).min(cols.saturating_sub(1));
                new_cursor_abs = Some((r, c));
            }
        }
        if out.is_empty() {
            out.push(Row::blank(cols, blank));
        }

        // 4. The bottom `rows` rows become the live screen; everything above
        //    flows into scrollback. The cursor sits at the content end (it was
        //    the last row included), so the bottom window always contains it.
        //    Pad below with blanks when the content is shorter than the screen.
        let (cur_abs_row, cur_abs_col) =
            new_cursor_abs.unwrap_or((out.len().saturating_sub(1), 0));

        let screen_start = out.len().saturating_sub(rows);
        let mut new_cells: Vec<Row> = out.split_off(screen_start);
        let new_scrollback: VecDeque<Row> = VecDeque::from(out);
        while new_cells.len() < rows {
            new_cells.push(Row::blank(cols, blank));
        }

        // Trim scrollback to its bound.
        let mut new_scrollback = new_scrollback;
        if self.scrollback_max != 0 {
            while new_scrollback.len() > self.scrollback_max {
                new_scrollback.pop_front();
            }
        }

        self.cells = new_cells;
        self.scrollback = new_scrollback;
        self.cols = cols;
        self.rows = rows;
        self.cursor_row = cur_abs_row.saturating_sub(screen_start).min(rows - 1);
        self.cursor_col = cur_abs_col.min(cols - 1);
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.view_offset = 0;
        self.saved = None;
        self.bump_dirty();
    }

    /// Place a printable char at the cursor and advance.
    pub fn put_char(&mut self, ch: char) {
        if self.cursor_col >= self.cols {
            // We filled the last column and another glyph arrived: this row
            // continues onto the next via auto-wrap. Tag it so resize can
            // rejoin the two halves into one logical line. (An explicit
            // newline instead would reach `linefeed` directly, leaving the
            // flag false — that's the soft-vs-hard distinction reflow needs.)
            self.cells[self.cursor_row].wrapped = true;
            self.linefeed();
            self.cursor_col = 0;
        }
        let (fg, bg) = if self.reverse {
            (self.current_bg, self.current_fg)
        } else {
            (self.current_fg, self.current_bg)
        };
        self.cells[self.cursor_row][self.cursor_col] = Cell { ch, fg, bg };
        self.cursor_col += 1;
        self.bump_dirty();
    }

    pub fn carriage_return(&mut self) {
        self.cursor_col = 0;
        self.bump_dirty();
    }

    pub fn linefeed(&mut self) {
        if self.cursor_row == self.scroll_bottom {
            let blank = self.blank_cell();
            let displaced = self.cells.remove(self.scroll_top);
            if self.region_is_full() {
                self.push_scrollback(displaced);
            }
            self.cells
                .insert(self.scroll_bottom, Row::blank(self.cols, blank));
        } else if self.cursor_row + 1 < self.rows {
            self.cursor_row += 1;
        }
        self.bump_dirty();
    }

    fn push_scrollback(&mut self, line: Row) {
        // Alt screen content (vim, htop, less) is transient — don't pollute
        // the main scrollback with it.
        if self.alt.is_some() {
            return;
        }
        self.scrollback.push_back(line);
        // If the user is scrolled up, advance view_offset so their visible
        // content stays anchored to the same absolute logical row.
        if self.view_offset > 0 {
            self.view_offset += 1;
        }
        // `scrollback_max == 0` means infinite — skip the trim loop entirely.
        if self.scrollback_max == 0 {
            return;
        }
        while self.scrollback.len() > self.scrollback_max {
            self.scrollback.pop_front();
        }
        if self.view_offset > self.scrollback.len() {
            self.view_offset = self.scrollback.len();
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            self.bump_dirty();
        }
    }

    pub fn tab(&mut self) {
        let next = ((self.cursor_col / 8) + 1) * 8;
        self.cursor_col = next.min(self.cols.saturating_sub(1));
        self.bump_dirty();
    }

    pub fn cursor_up(&mut self, n: usize) {
        self.cursor_row = self.cursor_row.saturating_sub(n);
        self.bump_dirty();
    }

    pub fn cursor_down(&mut self, n: usize) {
        let max = self.rows.saturating_sub(1);
        self.cursor_row = (self.cursor_row + n).min(max);
        self.bump_dirty();
    }

    pub fn cursor_forward(&mut self, n: usize) {
        let max = self.cols.saturating_sub(1);
        self.cursor_col = (self.cursor_col + n).min(max);
        self.bump_dirty();
    }

    pub fn cursor_back(&mut self, n: usize) {
        self.cursor_col = self.cursor_col.saturating_sub(n);
        self.bump_dirty();
    }

    /// CHA — 1-based column.
    pub fn cursor_to_column(&mut self, col_1based: usize) {
        let max = self.cols.saturating_sub(1);
        self.cursor_col = col_1based.saturating_sub(1).min(max);
        self.bump_dirty();
    }

    /// VPA — 1-based row.
    pub fn cursor_to_row(&mut self, row_1based: usize) {
        let max = self.rows.saturating_sub(1);
        self.cursor_row = row_1based.saturating_sub(1).min(max);
        self.bump_dirty();
    }

    /// CUP — 1-based row, col.
    pub fn set_cursor(&mut self, row_1based: usize, col_1based: usize) {
        let rmax = self.rows.saturating_sub(1);
        let cmax = self.cols.saturating_sub(1);
        self.cursor_row = row_1based.saturating_sub(1).min(rmax);
        self.cursor_col = col_1based.saturating_sub(1).min(cmax);
        self.bump_dirty();
    }

    pub fn erase_in_line(&mut self, mode: u16) {
        let row = self.cursor_row;
        let blank = self.blank_cell();
        match mode {
            0 => {
                for c in self.cursor_col..self.cols {
                    self.cells[row][c] = blank;
                }
            }
            1 => {
                let end = (self.cursor_col + 1).min(self.cols);
                for c in 0..end {
                    self.cells[row][c] = blank;
                }
            }
            2 => {
                for c in 0..self.cols {
                    self.cells[row][c] = blank;
                }
            }
            _ => {}
        }
        self.bump_dirty();
    }

    pub fn erase_in_display(&mut self, mode: u16) {
        let blank = self.blank_cell();
        match mode {
            0 => {
                self.erase_in_line(0);
                for r in (self.cursor_row + 1)..self.rows {
                    for c in 0..self.cols {
                        self.cells[r][c] = blank;
                    }
                }
            }
            1 => {
                for r in 0..self.cursor_row {
                    for c in 0..self.cols {
                        self.cells[r][c] = blank;
                    }
                }
                self.erase_in_line(1);
            }
            2 | 3 => {
                for r in 0..self.rows {
                    for c in 0..self.cols {
                        self.cells[r][c] = blank;
                    }
                }
            }
            _ => {}
        }
        self.bump_dirty();
    }

    pub fn delete_chars(&mut self, n: usize) {
        let row = self.cursor_row;
        let start = self.cursor_col;
        if start >= self.cols {
            return;
        }
        let n = n.min(self.cols - start);
        let blank = self.blank_cell();
        for c in start..(self.cols - n) {
            self.cells[row][c] = self.cells[row][c + n];
        }
        for c in (self.cols - n)..self.cols {
            self.cells[row][c] = blank;
        }
        self.bump_dirty();
    }

    pub fn insert_chars(&mut self, n: usize) {
        let row = self.cursor_row;
        let start = self.cursor_col;
        if start >= self.cols {
            return;
        }
        let n = n.min(self.cols - start);
        let blank = self.blank_cell();
        for c in (start + n..self.cols).rev() {
            self.cells[row][c] = self.cells[row][c - n];
        }
        for c in start..(start + n) {
            self.cells[row][c] = blank;
        }
        self.bump_dirty();
    }

    pub fn erase_chars(&mut self, n: usize) {
        let row = self.cursor_row;
        let start = self.cursor_col;
        let end = (start + n).min(self.cols);
        let blank = self.blank_cell();
        for c in start..end {
            self.cells[row][c] = blank;
        }
        self.bump_dirty();
    }

    pub fn scroll_up(&mut self, n: usize) {
        let region_rows = self.scroll_bottom - self.scroll_top + 1;
        let n = n.min(region_rows);
        let blank = self.blank_cell();
        let full = self.region_is_full();
        for _ in 0..n {
            let displaced = self.cells.remove(self.scroll_top);
            if full {
                self.push_scrollback(displaced);
            }
            self.cells
                .insert(self.scroll_bottom, Row::blank(self.cols, blank));
        }
        self.bump_dirty();
    }

    pub fn scroll_down(&mut self, n: usize) {
        let region_rows = self.scroll_bottom - self.scroll_top + 1;
        let n = n.min(region_rows);
        let blank = self.blank_cell();
        for _ in 0..n {
            self.cells.remove(self.scroll_bottom);
            self.cells
                .insert(self.scroll_top, Row::blank(self.cols, blank));
        }
        self.bump_dirty();
    }

    /// IL — insert `n` blank lines at the cursor row, shifting rows below
    /// downward within the current scroll region. Rows pushed past the bottom
    /// margin are discarded. No-op when the cursor is outside the region.
    pub fn insert_lines(&mut self, n: usize) {
        let row = self.cursor_row;
        if row < self.scroll_top || row > self.scroll_bottom {
            return;
        }
        let n = n.min(self.scroll_bottom - row + 1);
        let blank = self.blank_cell();
        for _ in 0..n {
            self.cells.remove(self.scroll_bottom);
            self.cells.insert(row, Row::blank(self.cols, blank));
        }
        self.bump_dirty();
    }

    /// DL — delete `n` lines starting at the cursor row, pulling rows below
    /// upward within the current scroll region; blanks fill in at the bottom
    /// margin. No-op when the cursor is outside the region.
    pub fn delete_lines(&mut self, n: usize) {
        let row = self.cursor_row;
        if row < self.scroll_top || row > self.scroll_bottom {
            return;
        }
        let n = n.min(self.scroll_bottom - row + 1);
        let blank = self.blank_cell();
        for _ in 0..n {
            self.cells.remove(row);
            self.cells
                .insert(self.scroll_bottom, Row::blank(self.cols, blank));
        }
        self.bump_dirty();
    }

    fn blank_cell(&self) -> Cell {
        let (fg, bg) = if self.reverse {
            (self.current_bg, self.current_fg)
        } else {
            (self.current_fg, self.current_bg)
        };
        Cell { ch: ' ', fg, bg }
    }

    fn bump_dirty(&mut self) {
        self.dirty = self.dirty.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_advances_cursor() {
        let mut g = Grid::new(10, 3);
        g.put_char('h');
        g.put_char('i');
        assert_eq!(g.cell(0, 0).ch, 'h');
        assert_eq!(g.cell(0, 1).ch, 'i');
        assert_eq!(g.cursor(), (0, 2));
    }

    #[test]
    fn wrap_to_next_row() {
        let mut g = Grid::new(3, 3);
        for c in "abcdef".chars() {
            g.put_char(c);
        }
        assert_eq!(g.cell(0, 0).ch, 'a');
        assert_eq!(g.cell(1, 0).ch, 'd');
        assert_eq!(g.cell(1, 2).ch, 'f');
    }

    #[test]
    fn linefeed_at_bottom_scrolls() {
        let mut g = Grid::new(3, 2);
        g.put_char('a');
        g.put_char('b');
        g.linefeed();
        g.carriage_return();
        g.put_char('c');
        g.put_char('d');
        g.linefeed();
        assert_eq!(g.cell(0, 0).ch, 'c');
        assert_eq!(g.cell(0, 1).ch, 'd');
        assert_eq!(g.cell(1, 0).ch, ' ');
        assert_eq!(g.cursor().0, 1);
    }

    #[test]
    fn erase_in_line_after_cursor() {
        let mut g = Grid::new(5, 1);
        for c in "abcde".chars() {
            g.put_char(c);
        }
        g.set_cursor(1, 3); // 1-based
        g.erase_in_line(0);
        let s: String = g.row(0).iter().map(|c| c.ch).collect();
        assert_eq!(s, "ab   ");
    }

    #[test]
    fn scrollback_max_zero_means_infinite() {
        let mut g = Grid::new(3, 2);
        g.set_scrollback_max(0);
        for _ in 0..50 {
            g.linefeed();
            g.carriage_return();
        }
        // Each linefeed at the bottom of a full-screen region scrolls one
        // blank line into history. With "infinite" set, none should be
        // dropped.
        assert_eq!(g.scrollback_len(), 49);
    }

    #[test]
    fn set_scrollback_max_to_zero_does_not_drop_existing() {
        let mut g = Grid::new(3, 2);
        // Fill with finite cap first.
        g.set_scrollback_max(10);
        for _ in 0..20 {
            g.linefeed();
            g.carriage_return();
        }
        assert_eq!(g.scrollback_len(), 10);
        // Switching to infinite must not wipe what we already had.
        g.set_scrollback_max(0);
        assert_eq!(g.scrollback_len(), 10);
    }

    #[test]
    fn scroll_pushes_to_scrollback_and_view_offset_shows_old_rows() {
        let mut g = Grid::new(3, 2);
        for c in "AB".chars() {
            g.put_char(c);
        }
        g.linefeed();
        g.carriage_return();
        for c in "CD".chars() {
            g.put_char(c);
        }
        g.linefeed(); // scrolls "AB" into scrollback
        g.carriage_return();
        for c in "EF".chars() {
            g.put_char(c);
        }
        assert_eq!(g.scrollback_len(), 1);
        // view_offset 0 → live screen: row 0 = "CD", row 1 = "EF"
        assert_eq!(g.visible_cell(0, 0).ch, 'C');
        assert_eq!(g.visible_cell(1, 0).ch, 'E');
        // view_offset 1 → top row from scrollback ("AB"), bottom row live ("CD")
        g.set_view_offset(1);
        assert_eq!(g.visible_cell(0, 0).ch, 'A');
        assert_eq!(g.visible_cell(0, 1).ch, 'B');
        assert_eq!(g.visible_cell(1, 0).ch, 'C');
    }

    #[test]
    fn linefeed_within_region_does_not_touch_rows_above_top() {
        let mut g = Grid::new(3, 5);
        // Fill rows with row-number markers so we can see what scrolled.
        for r in 0..5 {
            g.set_cursor(r + 1, 1);
            for c in 0..3 {
                g.put_char((b'0' + r as u8 + c as u8) as char);
            }
        }
        // Pin rows 0..=1 with scroll region [3..=5] (1-based).
        g.set_scroll_region(3, 5);
        // Move into the region's bottom margin and feed lines — header rows
        // (0,1) must stay untouched, region must scroll.
        g.set_cursor(5, 1);
        let header_before: Vec<char> = (0..3).map(|c| g.cell(0, c).ch).collect();
        let row1_before: Vec<char> = (0..3).map(|c| g.cell(1, c).ch).collect();
        for _ in 0..4 {
            g.linefeed();
        }
        let header_after: Vec<char> = (0..3).map(|c| g.cell(0, c).ch).collect();
        let row1_after: Vec<char> = (0..3).map(|c| g.cell(1, c).ch).collect();
        assert_eq!(header_before, header_after);
        assert_eq!(row1_before, row1_after);
        // Region rows should now all be blank — we scrolled enough times.
        for r in 2..5 {
            assert_eq!(g.row(r).iter().map(|c| c.ch).collect::<String>(), "   ");
        }
    }

    #[test]
    fn partial_region_scroll_does_not_pollute_scrollback() {
        let mut g = Grid::new(3, 4);
        g.set_scroll_region(2, 4);
        g.set_cursor(4, 1);
        for _ in 0..10 {
            g.linefeed();
        }
        assert_eq!(g.scrollback_len(), 0);
    }

    #[test]
    fn insert_and_delete_lines_within_region() {
        let mut g = Grid::new(2, 5);
        // Mark rows A..E.
        for (i, ch) in ['A', 'B', 'C', 'D', 'E'].into_iter().enumerate() {
            g.set_cursor(i + 1, 1);
            g.put_char(ch);
        }
        g.set_scroll_region(2, 4); // rows 1..=3 (0-based)
        g.set_cursor(2, 1); // cursor at row 1 (the 'B' row)
        g.insert_lines(1);
        // After IL: row0 = A (outside region), row1 = blank, row2 = B, row3 = C, row4 = E.
        // D was pushed past scroll_bottom and discarded.
        let col0: String = (0..5).map(|r| g.cell(r, 0).ch).collect();
        assert_eq!(col0, "A BCE");
        g.delete_lines(1);
        // DL removes the blank at row1, shifting B,C up; row3 fills blank.
        let col0: String = (0..5).map(|r| g.cell(r, 0).ch).collect();
        assert_eq!(col0, "ABC E");
    }

    #[test]
    fn set_scroll_region_reset_with_zeros() {
        let mut g = Grid::new(2, 4);
        g.set_scroll_region(2, 3);
        assert_eq!(g.scroll_region(), (1, 2));
        g.set_scroll_region(0, 0);
        assert_eq!(g.scroll_region(), (0, 3));
    }

    #[test]
    fn alt_screen_resets_scroll_region() {
        let mut g = Grid::new(2, 4);
        g.set_scroll_region(2, 3);
        // Simulate alt enter; region should snap back to full.
        g.enter_alt_screen();
        assert_eq!(g.scroll_region(), (0, 3));
        // Set a region inside alt, exit — should reset again.
        g.set_scroll_region(1, 2);
        g.exit_alt_screen();
        assert_eq!(g.scroll_region(), (0, 3));
    }

    #[test]
    fn delete_chars_shifts_left() {
        let mut g = Grid::new(6, 1);
        for c in "abcdef".chars() {
            g.put_char(c);
        }
        g.set_cursor(1, 2);
        g.delete_chars(2);
        let s: String = g.row(0).iter().map(|c| c.ch).collect();
        assert_eq!(s, "adef  ");
    }

    // ── reflow on resize ──────────────────────────────────────────────────

    fn type_str(g: &mut Grid, s: &str) {
        for ch in s.chars() {
            if ch == '\n' {
                g.linefeed();
                g.carriage_return();
            } else {
                g.put_char(ch);
            }
        }
    }

    fn row_str(g: &Grid, r: usize) -> String {
        g.row(r)
            .iter()
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn reflow_widening_rejoins_wrapped_line() {
        let mut g = Grid::new(10, 4);
        type_str(&mut g, "HELLOWORLD12");
        // Auto-wrapped at the 10-col edge.
        assert_eq!(row_str(&g, 0), "HELLOWORLD");
        assert_eq!(row_str(&g, 1), "12");

        g.resize(20, 4);
        // The two halves rejoin into one logical line at the wider width.
        assert_eq!(row_str(&g, 0), "HELLOWORLD12");
        assert_eq!(row_str(&g, 1), "");
        // Cursor stays attached to the same text position.
        assert_eq!(g.cursor(), (0, 12));
    }

    #[test]
    fn reflow_narrowing_splits_line() {
        let mut g = Grid::new(20, 4);
        type_str(&mut g, "HELLOWORLD12");
        assert_eq!(row_str(&g, 0), "HELLOWORLD12");

        g.resize(4, 4);
        assert_eq!(row_str(&g, 0), "HELL");
        assert_eq!(row_str(&g, 1), "OWOR");
        assert_eq!(row_str(&g, 2), "LD12");
    }

    #[test]
    fn reflow_preserves_hard_newlines() {
        let mut g = Grid::new(10, 5);
        type_str(&mut g, "line one\nline two");
        g.resize(40, 5);
        // A real newline must NOT be rejoined into the previous line.
        assert_eq!(row_str(&g, 0), "line one");
        assert_eq!(row_str(&g, 1), "line two");
    }

    #[test]
    fn reflow_rejoins_across_scrollback_boundary() {
        // A single 12-char auto-wrapped line, half scrolled into history.
        let mut g = Grid::new(4, 2);
        type_str(&mut g, "ABCDEFGHIJKL");
        assert_eq!(g.scrollback_len(), 1); // "ABCD" scrolled off
        assert_eq!(row_str(&g, 0), "EFGH");
        assert_eq!(row_str(&g, 1), "IJKL");

        g.resize(12, 4);
        // Widening pulls the whole logical line back onto one screen row.
        assert_eq!(g.scrollback_len(), 0);
        assert_eq!(row_str(&g, 0), "ABCDEFGHIJKL");
    }

    #[test]
    fn reflow_in_alt_screen_does_not_rejoin() {
        let mut g = Grid::new(4, 3);
        g.enter_alt_screen();
        type_str(&mut g, "ABCDEF"); // wraps within the alt screen
        // Alt screen is pad/truncate only — resizing must not panic or reflow.
        g.resize(8, 3);
        assert_eq!(g.cols(), 8);
        assert_eq!(g.rows(), 3);
        assert!(g.is_alt_screen());
    }
}
