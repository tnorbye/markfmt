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

//! Warnings about Markdown that probably does not render the way its author intended. These
//! never change the output; the formatter only reports them.

use pulldown_cmark::{Event, Options, Parser, Tag};

use crate::table;

/// A problem found in a document, reported as a warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// 1-based line number.
    pub line: usize,
    pub message: String,
}

/// Checks `input` for likely mistakes:
///
/// - A `|` inside a code span in a table row, which GFM treats as a cell separator unless it
///   is escaped as `\|`.
pub fn lint(input: &str) -> Vec<Warning> {
    if !input.contains('|') || !input.contains('`') {
        return Vec::new();
    }

    let mut warnings = Vec::new();
    let parser = Parser::new_ext(input, Options::ENABLE_TABLES).into_offset_iter();
    for (event, range) in parser {
        if !matches!(event, Event::Start(Tag::Table(_))) {
            continue;
        }
        let first_line = 1 + input[..range.start].bytes().filter(|&b| b == b'\n').count();
        for (offset, row) in input[range].lines().enumerate() {
            if !table::code_spans_with_pipes(row).is_empty() {
                warnings.push(Warning {
                    line: first_line + offset,
                    message: "`|` inside a code span splits this table cell; escape it as `\\|`"
                        .to_string(),
                });
            }
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::lint;

    #[test]
    fn warns_about_pipes_in_table_code_spans_only() {
        let input = "Text with `a|b` outside a table.\n\n| a | b |\n| - | - |\n| `x|y` | 2 |\n| `x\\|y` | 3 |\n";
        let warnings = lint(input);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].line, 5);
    }
}
