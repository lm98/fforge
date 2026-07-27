//! Column layout for the screens.
//!
//! Every screen used to hand-roll its `{:<22}` formatting, and Batch 4 adds
//! four more that would each hand-roll it again. Worse, hand-rolled widths and
//! colour do not mix: an escape sequence has zero visual width but several
//! bytes of it, so `format!("{:<20}", palette.paint(..))` silently pads to the
//! wrong place and the whole table shears one row at a time.
//!
//! **This module pads first and paints second**, which is the only ordering
//! that survives colour. `alignment_survives_colour` pins it: the coloured
//! render with its escapes stripped must equal the plain render, byte for
//! byte.
//!
//! Widths are counted in `char`s, matching what `{:<22}` already did — so
//! "Atlético Rivemona" occupies 17 columns, not its 18 bytes.

use crate::render::sem::{Palette, Sem};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// A column: its header label, its width, and which way its content sits.
/// A width of `0` means "don't pad" — the right shape for a trailing free-text
/// column like a `(shortlisted)` flag.
#[derive(Debug, Clone)]
pub struct Col {
    pub label: String,
    pub width: usize,
    pub align: Align,
}

impl Col {
    pub fn left(label: impl Into<String>, width: usize) -> Col {
        Col {
            label: label.into(),
            width,
            align: Align::Left,
        }
    }

    pub fn right(label: impl Into<String>, width: usize) -> Col {
        Col {
            label: label.into(),
            width,
            align: Align::Right,
        }
    }
}

/// One cell: text plus the semantic it carries. `Sem::Ok` (the default) renders
/// plain.
#[derive(Debug, Clone)]
pub struct Cell {
    pub text: String,
    pub sem: Sem,
}

impl Cell {
    pub fn new(text: impl Into<String>) -> Cell {
        Cell {
            text: text.into(),
            sem: Sem::Ok,
        }
    }

    /// Attach a semantic to *one* cell, where [`Table::row_all`]'s whole-row
    /// stamp is too coarse. Remember R15: whatever this colour says must also
    /// be said by a glyph, a column, or the row ordering.
    ///
    /// Only the tests reach it today; U4's finances screen — where the axis
    /// lands on individual figures rather than whole rows — is its first
    /// production consumer.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with(mut self, sem: Sem) -> Cell {
        self.sem = sem;
        self
    }
}

impl From<&str> for Cell {
    fn from(s: &str) -> Cell {
        Cell::new(s)
    }
}

impl From<String> for Cell {
    fn from(s: String) -> Cell {
        Cell::new(s)
    }
}

/// A row: its cells, plus an optional whole-row semantic. A uniform row is
/// painted as one span rather than cell-by-cell — same pixels, a fraction of
/// the escapes, and far easier to read in a `cat -v`.
#[derive(Debug, Clone)]
struct Row {
    cells: Vec<Cell>,
    uniform: Option<Sem>,
}

/// A column-aligned block: a header row followed by data rows.
#[derive(Debug, Clone)]
pub struct Table {
    cols: Vec<Col>,
    rows: Vec<Row>,
    /// Printed verbatim at the start of every line, header included — the
    /// single leading space most of these screens already indent by.
    indent: String,
    /// Whether to emit the header row at all.
    header: bool,
}

impl Table {
    pub fn new(cols: Vec<Col>) -> Table {
        Table {
            cols,
            rows: Vec::new(),
            indent: " ".to_string(),
            header: true,
        }
    }

    pub fn indent(mut self, indent: impl Into<String>) -> Table {
        self.indent = indent.into();
        self
    }

    /// Append a row. Extra cells beyond the declared columns are appended
    /// unpadded, which is what a trailing flag column wants.
    pub fn row(&mut self, cells: Vec<Cell>) {
        self.rows.push(Row {
            cells,
            uniform: None,
        });
    }

    /// Append a row whose whole line carries `sem` — the "this row is yours"
    /// case.
    pub fn row_all(&mut self, cells: Vec<Cell>, sem: Sem) {
        self.rows.push(Row {
            cells,
            uniform: Some(sem),
        });
    }

    /// Render to a newline-terminated block.
    pub fn render(&self, p: Palette) -> String {
        let mut out = String::new();
        if self.header {
            let header = Row {
                cells: self
                    .cols
                    .iter()
                    .map(|c| Cell::new(c.label.clone()))
                    .collect(),
                uniform: None,
            };
            let _ = writeln!(out, "{}", self.line(&header, p));
        }
        for row in &self.rows {
            let _ = writeln!(out, "{}", self.line(row, p));
        }
        out
    }

    fn line(&self, row: &Row, p: Palette) -> String {
        // Pad *then* paint — see the module docs.
        let mut padded: Vec<String> = row
            .cells
            .iter()
            .enumerate()
            .map(|(i, cell)| match self.cols.get(i) {
                Some(col) => pad(&cell.text, col.width, col.align),
                None => cell.text.clone(),
            })
            .collect();
        // Trailing padding is invisible either way, and dropping it keeps the
        // snapshots free of trailing whitespace. It has to happen *before*
        // painting: once a cell is painted, its trailing spaces sit inside the
        // escape pair and a `trim_end` on the finished line can no longer see
        // them — which is precisely the bug this ordering exists to avoid.
        while padded.last().is_some_and(|s| s.trim().is_empty()) {
            padded.pop();
        }
        if let Some(last) = padded.last_mut() {
            let trimmed = last.trim_end().to_string();
            *last = trimmed;
        }
        if padded.is_empty() {
            return self.indent.trim_end().to_string();
        }
        let body = match row.uniform {
            Some(sem) => p.paint(&padded.join(" "), sem),
            None => padded
                .iter()
                .zip(&row.cells)
                .map(|(text, cell)| p.paint(text, cell.sem))
                .collect::<Vec<_>>()
                .join(" "),
        };
        format!("{}{}", self.indent, body)
    }
}

/// Pad `text` to `width` *characters* (not bytes). Over-long text is never
/// truncated — a clipped player name is worse than a shifted column, and the
/// widths here are chosen with room to spare.
pub fn pad(text: &str, width: usize, align: Align) -> String {
    let len = text.chars().count();
    if len >= width {
        return text.to_string();
    }
    let fill = " ".repeat(width - len);
    match align {
        Align::Left => format!("{text}{fill}"),
        Align::Right => format!("{fill}{text}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                // Consume the CSI sequence up to and including its final byte.
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn sample() -> Table {
        let mut t = Table::new(vec![
            Col::left("Club", 22),
            Col::right("Pts", 4),
            Col::left("", 0),
        ]);
        t.row(vec![
            Cell::new("Atlético Rivemona").with(Sem::Warn),
            Cell::new("43").with(Sem::Bad),
            Cell::new("(relegation form)").with(Sem::Muted),
        ]);
        t.row_all(
            vec![Cell::new("FC Nerana"), Cell::new("65"), Cell::new("")],
            Sem::Mine,
        );
        t
    }

    /// The reason this module exists. Colour must be invisible to layout.
    #[test]
    fn alignment_survives_colour() {
        let t = sample();
        assert_eq!(
            strip_ansi(&t.render(Palette::COLOURED)),
            t.render(Palette::PLAIN)
        );
    }

    #[test]
    fn widths_count_characters_not_bytes() {
        // "Atlético" is 8 chars but 9 bytes; padding to 10 must add 2 spaces.
        assert_eq!(pad("Atlético", 10, Align::Left), "Atlético  ");
        assert_eq!(pad("Atlético", 10, Align::Right), "  Atlético");
    }

    #[test]
    fn over_long_content_is_never_truncated() {
        assert_eq!(
            pad("Olimpia Veraverde", 4, Align::Left),
            "Olimpia Veraverde"
        );
    }

    #[test]
    fn the_plain_render_has_no_escapes_and_no_trailing_space() {
        let rendered = sample().render(Palette::PLAIN);
        assert!(!rendered.contains('\u{1b}'));
        for line in rendered.lines() {
            assert_eq!(line, line.trim_end(), "trailing whitespace in {line:?}");
        }
    }

    /// Character offset of `needle`'s last character in `haystack` — byte
    /// offsets would be one out on any line containing "Atlético", which is
    /// the whole reason `pad` counts characters.
    fn char_end_of(haystack: &str, needle: &str) -> usize {
        let byte = haystack.find(needle).expect("needle present");
        haystack[..byte].chars().count() + needle.chars().count()
    }

    #[test]
    fn columns_line_up() {
        let rendered = sample().render(Palette::PLAIN);
        let lines: Vec<&str> = rendered.lines().collect();
        // Header + two rows.
        assert_eq!(lines.len(), 3);
        // The right-aligned points column ends at the same offset on every line.
        assert_eq!(char_end_of(lines[0], "Pts"), 28);
        assert_eq!(char_end_of(lines[1], "43"), 28);
        assert_eq!(char_end_of(lines[2], "65"), 28);
    }
}
