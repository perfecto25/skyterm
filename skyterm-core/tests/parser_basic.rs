use skyterm_core::{Grid, Parser};

fn row_str(grid: &Grid, row: usize) -> String {
    grid.row(row)
        .iter()
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn shell_prompt_then_command_output() {
    let mut grid = Grid::new(40, 5);
    let mut parser = Parser::new();
    // Simulated shell session bytes
    parser.advance(&mut grid, b"$ ls\r\nfoo  bar  baz\r\n$ ");
    assert_eq!(row_str(&grid, 0), "$ ls");
    assert_eq!(row_str(&grid, 1), "foo  bar  baz");
    assert_eq!(row_str(&grid, 2), "$");
}

#[test]
fn long_output_scrolls() {
    let mut grid = Grid::new(20, 3);
    let mut parser = Parser::new();
    parser.advance(&mut grid, b"line1\r\nline2\r\nline3\r\nline4");
    assert_eq!(row_str(&grid, 0), "line2");
    assert_eq!(row_str(&grid, 1), "line3");
    assert_eq!(row_str(&grid, 2), "line4");
}
