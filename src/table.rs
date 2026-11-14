// Copyright 2026 Tor Norbye
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableAlign {
    Unspecified,
    LeftColon,
    Center,
    Right,
}

impl TableAlign {
    fn min_divider_width(self) -> usize {
        match self {
            TableAlign::Unspecified => 3,
            TableAlign::LeftColon | TableAlign::Right => 4,
            TableAlign::Center => 5,
        }
    }
}

/// Formats a GFM Markdown table with uniform 1-space cell padding (`| col |`) and
/// Unicode display-width column alignment. Returns `None` if `raw_table` is not a valid GFM table.
pub fn format_table(raw_table: &str, indent_prefix: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = raw_table
        .lines()
        .map(|l| l.trim_end_matches([' ', '\t', '\r']))
        .filter(|l| !l.trim().is_empty())
        .collect();

    if lines.len() < 2 {
        return None;
    }

    let mut rows: Vec<Vec<String>> = Vec::with_capacity(lines.len());
    for line in &lines {
        let cells = parse_table_row(line)?;
        if cells.is_empty() {
            return None;
        }
        rows.push(cells);
    }

    if rows[0].len() != rows[1].len() {
        return None;
    }

    let mut aligns = Vec::with_capacity(rows[1].len());
    for cell in &rows[1] {
        aligns.push(parse_alignment_cell(cell)?);
    }

    // GFM ignores body cells beyond the header's column count. Aligning them would require adding
    // columns to the header and delimiter rows, which changes the rendered table, so leave such
    // tables alone.
    let num_cols = rows[0].len();
    if num_cols == 0 || rows.iter().any(|r| r.len() > num_cols) {
        return None;
    }
    for row in &mut rows {
        while row.len() < num_cols {
            row.push(String::new());
        }
    }

    let mut widths: Vec<usize> = aligns
        .iter()
        .map(|a| a.min_divider_width().max(3))
        .collect();

    for (row_idx, row) in rows.iter().enumerate() {
        if row_idx == 1 {
            continue;
        }
        for (col_idx, cell) in row.iter().enumerate() {
            let w = UnicodeWidthStr::width(cell.as_str());
            if w > widths[col_idx] {
                widths[col_idx] = w;
            }
        }
    }

    let mut out = Vec::with_capacity(rows.len());
    for (row_idx, row) in rows.iter().enumerate() {
        let mut line = String::from(indent_prefix);
        if row_idx == 1 {
            for (col_idx, &align) in aligns.iter().enumerate() {
                let w = widths[col_idx];
                line.push_str("| ");
                match align {
                    TableAlign::Unspecified => {
                        for _ in 0..w {
                            line.push('-');
                        }
                    }
                    TableAlign::LeftColon => {
                        line.push(':');
                        for _ in 0..(w - 1) {
                            line.push('-');
                        }
                    }
                    TableAlign::Right => {
                        for _ in 0..(w - 1) {
                            line.push('-');
                        }
                        line.push(':');
                    }
                    TableAlign::Center => {
                        line.push(':');
                        for _ in 0..(w - 2) {
                            line.push('-');
                        }
                        line.push(':');
                    }
                }
                line.push(' ');
            }
            line.push('|');
        } else {
            for (col_idx, cell) in row.iter().enumerate() {
                let target_w = widths[col_idx];
                let cell_w = UnicodeWidthStr::width(cell.as_str());
                let pad = target_w.saturating_sub(cell_w);
                line.push_str("| ");
                if row_idx == 0 {
                    line.push_str(cell);
                    push_spaces(&mut line, pad);
                } else {
                    match aligns[col_idx] {
                        TableAlign::Unspecified | TableAlign::LeftColon => {
                            line.push_str(cell);
                            push_spaces(&mut line, pad);
                        }
                        TableAlign::Right => {
                            push_spaces(&mut line, pad);
                            line.push_str(cell);
                        }
                        TableAlign::Center => {
                            let left = pad / 2;
                            let right = pad - left;
                            push_spaces(&mut line, left);
                            line.push_str(cell);
                            push_spaces(&mut line, right);
                        }
                    }
                }
                line.push(' ');
            }
            line.push('|');
        }
        out.push(line);
    }

    Some(out)
}

fn push_spaces(s: &mut String, count: usize) {
    for _ in 0..count {
        s.push(' ');
    }
}

fn parse_alignment_cell(cell: &str) -> Option<TableAlign> {
    let t = cell.trim();
    if t.is_empty() {
        return None;
    }
    let left_colon = t.starts_with(':');
    let right_colon = t.ends_with(':');
    let inner = t.strip_prefix(':').unwrap_or(t);
    let inner = inner.strip_suffix(':').unwrap_or(inner);
    if inner.is_empty() || !inner.bytes().all(|b| b == b'-') {
        return None;
    }
    Some(match (left_colon, right_colon) {
        (true, true) => TableAlign::Center,
        (false, true) => TableAlign::Right,
        (true, false) => TableAlign::LeftColon,
        (false, false) => TableAlign::Unspecified,
    })
}

/// Splits a table row into trimmed cell strings at unescaped `|`, exactly like GFM: a `|` inside
/// a code span still separates cells unless it is escaped as `\|`.
fn parse_table_row(raw_line: &str) -> Option<Vec<String>> {
    let trimmed = raw_line.trim();
    if !trimmed.contains('|') {
        return None;
    }

    let bytes = trimmed.as_bytes();
    let mut pipe_positions = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'|' {
            pipe_positions.push(i);
        }
        i += 1;
    }

    if pipe_positions.is_empty() {
        return None;
    }

    let mut cells = Vec::new();
    let start_idx = if pipe_positions.first() == Some(&0) {
        1
    } else {
        0
    };
    let end_idx =
        if pipe_positions.last() == Some(&(bytes.len() - 1)) && pipe_positions.len() > start_idx {
            pipe_positions.len() - 1
        } else {
            pipe_positions.len()
        };

    let mut prev = if start_idx == 1 { 1 } else { 0 };
    for &p in &pipe_positions[start_idx..end_idx] {
        cells.push(trimmed[prev..p].trim().to_string());
        prev = p + 1;
    }

    let tail_end = if end_idx < pipe_positions.len() {
        pipe_positions[end_idx]
    } else {
        bytes.len()
    };
    if prev <= tail_end {
        let last_cell = trimmed[prev..tail_end].trim();
        if !last_cell.is_empty() || !cells.is_empty() || end_idx == pipe_positions.len() {
            cells.push(last_cell.to_string());
        }
    }

    Some(cells)
}

/// Returns the 0-based byte offsets (within `line`) of code spans that contain an unescaped `|`.
/// In a GFM table such a pipe splits the cell, which is almost never what the author meant.
pub(crate) fn code_spans_with_pipes(line: &str) -> Vec<usize> {
    let bytes = line.as_bytes();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
        let Some(close_end) = crate::inline::find_closing_backticks(line, i + run, run) else {
            i += run;
            continue;
        };
        let inner = &bytes[i + run..close_end - run];
        let has_unescaped_pipe = inner
            .iter()
            .enumerate()
            .any(|(k, &b)| b == b'|' && (k == 0 || inner[k - 1] != b'\\'));
        if has_unescaped_pipe {
            found.push(i);
        }
        i = close_end;
    }
    found
}
