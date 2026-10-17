//! Finding the markdown tables whose header starts with `ID`, and splitting their rows
//! into cells.
//!
//! A deliberate limitation: fenced code blocks are skipped by toggling on any line that
//! starts with three backticks or three tildes, without matching the fence kind or
//! length. The tracker has no nested fences.

const ID_COLUMN: &str = "ID";
const STATUS_COLUMN: &str = "Status";
const SEVERITY_COLUMN: &str = "Sev";

/// What a table's header says its rows look like.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TableSchema {
    pub columns: usize,
    pub status_column: Option<usize>,
    pub severity_column: Option<usize>,
}

impl TableSchema {
    fn from_header(header: &[String]) -> Self {
        let position = |name: &str| header.iter().position(|cell| cell == name);
        Self {
            columns: header.len(),
            status_column: position(STATUS_COLUMN),
            severity_column: position(SEVERITY_COLUMN),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TableRow {
    /// 0-based line index in the file.
    pub index: usize,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IdTable {
    pub schema: TableSchema,
    pub rows: Vec<TableRow>,
}

/// Every ID table in `text`. Its rows follow the `|---|` separator and end at the first
/// line that does not start with `|`.
pub(super) fn id_tables(text: &str) -> Vec<IdTable> {
    let mut lines = text.lines().enumerate().peekable();
    let mut tables = Vec::new();
    let mut in_fence = false;
    while let Some((_, line)) = lines.next() {
        if is_fence(line) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let Some(header) = id_header(line) else {
            continue;
        };
        if lines.next_if(|(_, next)| is_separator(next)).is_none() {
            continue;
        }
        let mut rows = Vec::new();
        while let Some((index, row)) = lines.next_if(|(_, next)| is_table_line(next)) {
            rows.push(TableRow {
                index,
                cells: split_cells(row),
            });
        }
        tables.push(IdTable {
            schema: TableSchema::from_header(&header),
            rows,
        });
    }
    tables
}

fn id_header(line: &str) -> Option<Vec<String>> {
    if !is_table_line(line) {
        return None;
    }
    let cells = split_cells(line);
    (cells.first().map(String::as_str) == Some(ID_COLUMN)).then_some(cells)
}

fn is_fence(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("```") || line.starts_with("~~~")
}

fn is_table_line(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

fn is_separator(line: &str) -> bool {
    let cells = split_cells(line);
    is_table_line(line)
        && !cells.is_empty()
        && cells.iter().map(String::as_str).all(is_separator_cell)
}

/// `---`, `:--`, `--:` or `:-:`.
fn is_separator_cell(cell: &str) -> bool {
    let dashes = cell.strip_prefix(':').unwrap_or(cell);
    let dashes = dashes.strip_suffix(':').unwrap_or(dashes);
    !dashes.is_empty() && dashes.chars().all(|c| c == '-')
}

/// The trimmed cells of a table line. Every unescaped `|` ends a cell, inside an inline
/// code span too, which is how GFM renders a table. `\|` is a literal pipe.
fn split_cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let body = line.strip_prefix('|').unwrap_or(line);
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cell.push('|');
            }
            '|' => cells.push(std::mem::take(&mut cell).trim().to_owned()),
            _ => cell.push(c),
        }
    }
    // What follows the last pipe is empty when the line has a closing pipe.
    if !cell.trim().is_empty() {
        cells.push(cell.trim().to_owned());
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_split_on_every_unescaped_pipe_including_inside_code_spans() {
        let line = r"| PP-01 | a \| b | `x \| y` | `p | q` | z |";

        let cells = split_cells(line);

        assert_eq!(cells, ["PP-01", "a | b", "`x | y`", "`p", "q`", "z"]);
    }

    #[test]
    fn rows_without_a_closing_pipe_keep_their_last_cell() {
        let cells = split_cells("| a | b | ");

        assert_eq!(cells, ["a", "b"]);
        assert_eq!(split_cells("| a | b"), ["a", "b"]);
        assert_eq!(split_cells("| a |  |"), ["a", ""]);
    }
}
