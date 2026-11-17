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

use crate::markdown::{parser_options, TAB_STOP};
use crate::options::FormattingOptions;
use crate::preprocess;
use crate::table;

/// A problem found in a document, reported as a warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// 1-based line number.
    pub line: usize,
    pub message: String,
}

/// Checks `input` for likely mistakes using default options:
///
/// - A `|` inside a code span in a table row, which GFM treats as a cell separator unless it
///   is escaped as `\|`.
pub fn lint(input: &str) -> Vec<Warning> {
    lint_with_options(input, &FormattingOptions::default())
}

/// Checks `input` for likely mistakes under `options`:
///
/// - A `|` inside a code span in a table row.
/// - In commit-message mode: a Git/Gerrit trailer (such as `Change-Id:`) attached directly to
///   preceding non-trailer text without a separating blank line, where `git interpret-trailers`
///   will ignore it. Comment lines and `git commit -v` scissors diffs are skipped.
pub fn lint_with_options(input: &str, options: &FormattingOptions) -> Vec<Warning> {
    // A byte order mark is not content (formatting keeps it aside too); line numbers are unchanged.
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    if !options.commit_message {
        return lint_table_pipes(input, None);
    }

    let normalized = input.replace("\r\n", "\n");
    let comment_prefix =
        preprocess::resolve_comment_prefix(&normalized, options.comment_char.as_str());
    let split = preprocess::split_commit_tail(&normalized, comment_prefix);
    let head = &normalized[..split.head_end];

    let commit = preprocess::CommitSyntax {
        comment_prefix,
        trailer_separators: &options.trailer_separators,
        trailer_keys: &options.trailer_keys,
    };
    let mut warnings = lint_table_pipes(head, Some(comment_prefix));
    // Git only looks for trailers above the `---` patch divider.
    let trailer_text = &head[..preprocess::git_trailer_search_end(head)];
    warnings.extend(lint_unseparated_trailers(trailer_text, commit));
    warnings.sort_by_key(|w| w.line);
    warnings
}

fn lint_table_pipes(input: &str, skip_comment_prefix: Option<&str>) -> Vec<Warning> {
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
            if skip_comment_prefix
                .is_some_and(|prefix| preprocess::is_commit_comment_line(row, prefix))
            {
                continue;
            }
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

fn lint_unseparated_trailers(head: &str, commit: preprocess::CommitSyntax<'_>) -> Vec<Warning> {
    let separators = commit.trailer_separators;
    if head.trim().is_empty() {
        return Vec::new();
    }

    // Ignore code blocks and HTML blocks so examples mentioning `Change-Id:` do not warn.
    let mut code_ranges = Vec::new();
    for (event, range) in Parser::new_ext(head, parser_options()).into_offset_iter() {
        if matches!(
            event,
            Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock) | Event::Code(_)
        ) {
            code_ranges.push(range);
        }
    }

    let raw_lines: Vec<&str> = head
        .strip_suffix('\n')
        .unwrap_or(head)
        .split('\n')
        .collect();
    let mut line_starts = Vec::with_capacity(raw_lines.len());
    let mut offset = 0usize;
    for &line in &raw_lines {
        line_starts.push(offset);
        offset += line.len() + 1;
    }

    // Group non-blank lines into paragraphs `(start_idx..=end_idx, preceded_by_blank)`.
    let mut paragraphs: Vec<(usize, usize, bool)> = Vec::new();
    let mut saw_blank_since_prev = false;
    let mut i = 0usize;
    while i < raw_lines.len() {
        if raw_lines[i].trim().is_empty() {
            if !paragraphs.is_empty() {
                saw_blank_since_prev = true;
            }
            i += 1;
            continue;
        }
        let start = i;
        while i + 1 < raw_lines.len() && !raw_lines[i + 1].trim().is_empty() {
            i += 1;
        }
        let preceded_by_blank = !paragraphs.is_empty() && saw_blank_since_prev;
        paragraphs.push((start, i, preceded_by_blank));
        saw_blank_since_prev = false;
        i += 1;
    }

    let git_block = preprocess::find_git_trailer_candidate(&raw_lines, commit)
        .filter(|c| c.is_block)
        .map(|c| c.lines);

    let mut warnings = Vec::new();
    for &(start, end, preceded_by_blank) in &paragraphs {
        if start == end {
            continue;
        }
        let slice = &raw_lines[start..=end];
        let in_code = (start..=end).any(|line_idx| {
            let byte_pos = line_starts[line_idx];
            let (cols, _) = preprocess::leading_ws_columns(raw_lines[line_idx], TAB_STOP);
            cols >= 4
                || code_ranges
                    .iter()
                    .any(|r| r.start <= byte_pos && byte_pos < r.end)
        });
        if in_code {
            continue;
        }

        // Lines Git reads as trailers (`git interpret-trailers --parse`) need no warning.
        let in_git_block = git_block.as_ref().is_some_and(|block| {
            (start..=end).all(|i| {
                block.contains(&i)
                    || raw_lines[i].trim().is_empty()
                    || preprocess::is_commit_comment_line(raw_lines[i], commit.comment_prefix)
            })
        });
        if in_git_block {
            continue;
        }
        if preceded_by_blank && preprocess::is_structured_trailer_entries(slice, commit) {
            continue;
        }

        let mut saw_non_trailer = false;
        for (rel, &line) in slice.iter().enumerate() {
            if preprocess::is_commit_comment_line(line, commit.comment_prefix) {
                continue;
            }
            if rel > 0
                && saw_non_trailer
                && preprocess::is_well_known_trailer_line(line, separators)
            {
                let label = preprocess::find_git_trailer_separator(line, separators)
                    .map(|(tok, sep)| {
                        format!("`{tok}{}` ", line[sep..].chars().next().unwrap_or(':'))
                    })
                    .unwrap_or_default();
                warnings.push(Warning {
                    line: start + rel + 1,
                    message: format!(
                        "trailer {label}is not separated from the preceding text by a blank line"
                    ),
                });
                break;
            }
            if !preprocess::is_well_known_trailer_line(line, separators) || !preceded_by_blank {
                saw_non_trailer = true;
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
