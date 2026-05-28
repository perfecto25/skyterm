use std::collections::VecDeque;

use crate::theme::CellColor;

const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

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

/// Fixed-size character grid with a wrapping cursor and current-SGR state.
///
/// M2 scope so far: cursor placement, erase, insert/delete chars, scroll
/// (no per-region scroll yet — full-screen), per-cell fg/bg, bold tracked but
/// not yet rendered. No scrollback.
pub struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<Vec<Cell>>,
    cursor_row: usize,
    cursor_col: usize,
    current_fg: CellColor,
    current_bg: CellColor,
    bold: bool,
    reverse: bool,
    cursor_visible: bool,
    saved: Option<SavedCursor>,
    /// When `Some`, the live screen is the alt screen and the saved main
    /// screen state lives here, waiting to be restored on exit.
    alt: Option<AltScreenSave>,
    scrollback: VecDeque<Vec<Cell>>,
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
    cells: Vec<Vec<Cell>>,
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
        let cells = vec![vec![Cell::EMPTY; cols]; rows];
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
            saved: None,
            alt: None,
            scrollback: VecDeque::new(),
            scrollback_max: DEFAULT_SCROLLBACK_LINES,
            view_offset: 0,
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
        let blank = Cell::EMPTY;
        let fresh = vec![vec![blank; self.cols]; self.rows];
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
                cells.resize_with(self.rows, || vec![Cell::EMPTY; self.cols]);
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

    /// Cell at view-relative position (0 = topmost visible row).
    /// When scrolled up, the top of the view shows old scrollback lines.
    pub fn visible_cell(&self, view_row: usize, col: usize) -> Cell {
        let sb_len = self.scrollback.len();
        let logical = sb_len.saturating_sub(self.view_offset) + view_row;
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
        &self.cells[row]
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
        let blank = self.blank_cell();
        self.cells.resize_with(rows, || vec![blank; cols]);
        for line in &mut self.cells {
            line.resize(cols, blank);
        }
        self.cols = cols;
        self.rows = rows;
        self.cursor_row = self.cursor_row.min(rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(cols.saturating_sub(1));
        self.scroll_top = 0;
        self.scroll_bottom = rows.saturating_sub(1);
        self.bump_dirty();
    }

    /// Place a printable char at the cursor and advance.
    pub fn put_char(&mut self, ch: char) {
        if self.cursor_col >= self.cols {
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
                .insert(self.scroll_bottom, vec![blank; self.cols]);
        } else if self.cursor_row + 1 < self.rows {
            self.cursor_row += 1;
        }
        self.bump_dirty();
    }

    fn push_scrollback(&mut self, line: Vec<Cell>) {
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
                .insert(self.scroll_bottom, vec![blank; self.cols]);
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
                .insert(self.scroll_top, vec![blank; self.cols]);
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
            self.cells.insert(row, vec![blank; self.cols]);
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
                .insert(self.scroll_bottom, vec![blank; self.cols]);
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
}
