//! Reverse video (SGR 7) over default colors must be preserved as a per-cell
//! flag, not pre-swapped at write time. ncurses apps (ncdu, less) highlight the
//! selected row with `ESC[7m` using the terminal's *default* fg/bg; swapping
//! Default↔Default at write time is a no-op, so the highlight vanished and
//! moving the selection looked like nothing happened. Regression for that.

use skyterm_core::grid::Grid;
use skyterm_core::parser::Parser;

#[test]
fn reverse_video_over_default_colors_sets_inverse_flag() {
    let mut g = Grid::new(20, 3);
    let mut p = Parser::new();
    // Plain text, then reverse-video text, then back to normal — all in the
    // default color, exactly how ncdu draws a selected row.
    p.advance(&mut g, b"ab\x1b[7mXY\x1b[27mcd");

    let inv: Vec<bool> = (0..6).map(|c| g.visible_cell(0, c).inverse).collect();
    assert_eq!(
        inv,
        vec![false, false, true, true, false, false],
        "only the SGR-7 span should carry the inverse flag"
    );
    // The reverse cells keep default colors — the swap happens at render time.
    let cell = g.visible_cell(0, 2);
    assert!(cell.inverse);
    assert_eq!(cell.ch, 'X');
}

#[test]
fn sgr_reset_clears_inverse() {
    let mut g = Grid::new(10, 1);
    let mut p = Parser::new();
    p.advance(&mut g, b"\x1b[7mA\x1b[0mB");
    assert!(g.visible_cell(0, 0).inverse);
    assert!(!g.visible_cell(0, 1).inverse, "SGR 0 must clear reverse");
}

#[test]
fn reverse_erase_to_eol_carries_highlight() {
    // ncdu ends a selected-row redraw with ESC[K while reverse is still active;
    // the cleared span must stay highlighted (background-color erase).
    let mut g = Grid::new(8, 1);
    let mut p = Parser::new();
    p.advance(&mut g, b"\x1b[7mHi\x1b[K");
    for c in 0..8 {
        assert!(g.visible_cell(0, c).inverse, "col {c} should be reverse-erased");
    }
}
