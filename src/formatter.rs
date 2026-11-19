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

use std::ops::Range;
use std::path::Path;

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use unicode_width::UnicodeWidthStr;

use crate::inline;
use crate::markdown::{follows_soft_break, parser_options, soft_breaks, TAB_STOP};
use crate::options::{FormattingOptions, OrderedListStyle};
use crate::preprocess::{self, dedent_columns, PreSegment};
use crate::range_filter::RangeFilter;
use crate::reflow;
use crate::table;

#[derive(Debug, Clone)]
struct BlockUnit {
    lines: Vec<String>,
    start_line: usize,
    end_line: usize,
    tight_before: bool,
    tight_after: bool,
    is_paragraph: bool,
    is_list: bool,
    is_standalone: bool,
    /// Copied verbatim (code, HTML, ignored regions): its lines already carry their indentation
    /// relative to the container's content column, so it must not take a wider hanging indent
    /// (that would shift indented code content and drift further on every run).
    keeps_source_indent: bool,
}

impl BlockUnit {
    /// A non-paragraph, non-list unit separated from its neighbours by blank lines.
    fn new(lines: Vec<String>, start_line: usize, end_line: usize) -> Self {
        Self {
            lines,
            start_line,
            end_line,
            tight_before: false,
            tight_after: false,
            is_paragraph: false,
            is_list: false,
            is_standalone: false,
            keeps_source_indent: false,
        }
    }

    /// A unit whose lines are emitted exactly as in the source.
    fn verbatim(lines: Vec<String>, start_line: usize, end_line: usize) -> Self {
        Self {
            keeps_source_indent: true,
            ..Self::new(lines, start_line, end_line)
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FirstParaOverride<'a> {
    first_prefix: &'a str,
    first_width: usize,
    rest_indent: usize,
    rest_width: usize,
    root_continuations: bool,
}

#[derive(Debug, Clone)]
enum TopAstItem {
    Paragraph {
        range: Range<usize>,
        follows_definition: bool,
    },
    Heading(Range<usize>),
    CodeBlock {
        kind_is_indented: bool,
        range: Range<usize>,
    },
    HtmlBlock(Range<usize>),
    Rule(Range<usize>),
    Table(Range<usize>),
    BlockQuote {
        is_alert: bool,
        range: Range<usize>,
    },
    List {
        range: Range<usize>,
        items: Vec<Range<usize>>,
        preserve_layout: bool,
    },
    Footnote {
        label: String,
        range: Range<usize>,
    },
    VerbatimBlock(Range<usize>),
}

impl TopAstItem {
    fn range(&self) -> Range<usize> {
        match self {
            TopAstItem::Paragraph { range: r, .. }
            | TopAstItem::Heading(r)
            | TopAstItem::CodeBlock { range: r, .. }
            | TopAstItem::HtmlBlock(r)
            | TopAstItem::Rule(r)
            | TopAstItem::Table(r)
            | TopAstItem::BlockQuote { range: r, .. }
            | TopAstItem::List { range: r, .. }
            | TopAstItem::Footnote { range: r, .. }
            | TopAstItem::VerbatimBlock(r) => r.clone(),
        }
    }

    fn set_start(&mut self, new_start: usize) {
        match self {
            TopAstItem::Paragraph { range: r, .. }
            | TopAstItem::Heading(r)
            | TopAstItem::CodeBlock { range: r, .. }
            | TopAstItem::HtmlBlock(r)
            | TopAstItem::Rule(r)
            | TopAstItem::Table(r)
            | TopAstItem::BlockQuote { range: r, .. }
            | TopAstItem::List { range: r, .. }
            | TopAstItem::Footnote { range: r, .. }
            | TopAstItem::VerbatimBlock(r) => r.start = new_start,
        }
    }
}

/// Formats a complete Markdown (or Markdeep) document.
pub fn format_markdown(input: &str, options: &FormattingOptions) -> String {
    format_file_content(input, Path::new(""), &RangeFilter::All, options)
}

/// Formats `input`, the contents of the file at `canonical_path`. If `range_filter` selects line
/// ranges, only the top-level blocks overlapping them are reformatted and everything else is
/// kept byte-for-byte.
///
/// A commit message comes back unchanged if formatting it would do more than change whitespace,
/// change the trailers Git parses, or give a result that formats differently a second time.
pub fn format_file_content(
    input: &str,
    canonical_path: &Path,
    range_filter: &RangeFilter,
    options: &FormattingOptions,
) -> String {
    let out = format_file_content_unchecked(input, canonical_path, range_filter, options);
    if !options.commit_message || out == input {
        return out;
    }
    // Partial formatting is safe only if formatting the whole message is.
    if range_filter.is_filtered() {
        let whole =
            format_file_content_unchecked(input, canonical_path, &RangeFilter::All, options);
        if !commit_format_is_safe(input, &whole, options, true) {
            return input.to_string();
        }
    }
    // Reformatting partial output would also format the lines outside the range, and the
    // range's line numbers no longer match after reflow, so idempotence is checked above on
    // the whole message instead.
    if commit_format_is_safe(input, &out, options, !range_filter.is_filtered()) {
        out
    } else {
        input.to_string()
    }
}

/// Whether `formatted` (fully formatted from `input`) keeps a commit message's meaning: the same
/// words in the same order, the same trailers as Git parses them, the same rendered structure,
/// and no further change when formatted again. Commit messages are read as plain text, so these hold them to more than
/// Markdown rendering does; this check is the backstop for cases the formatter misses.
fn commit_format_is_safe(
    input: &str,
    formatted: &str,
    options: &FormattingOptions,
    check_idempotence: bool,
) -> bool {
    let normalize = |text: &str| {
        text.strip_prefix('\u{feff}')
            .unwrap_or(text)
            .replace("\r\n", "\n")
    };
    // The byte order mark is not whitespace, so compare words only after removing it.
    let (before, after) = (normalize(input), normalize(formatted));
    // Renumbering ordered lists is the one non-whitespace change asked for explicitly.
    let renumbers = options.ordered_lists != OrderedListStyle::Preserve;
    let is_list_number = |word: &str| {
        word.strip_suffix(['.', ')'])
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
    };
    let same_word =
        |a: &str, b: &str| a == b || (renumbers && is_list_number(a) && is_list_number(b));
    let (mut words_before, mut words_after) = (before.split_whitespace(), after.split_whitespace());
    loop {
        match (words_before.next(), words_after.next()) {
            (None, None) => break,
            (Some(a), Some(b)) if same_word(a, b) => {}
            _ => return false,
        }
    }
    let comment_prefix = preprocess::resolve_comment_prefix(&before, &options.comment_char);
    let commit = preprocess::CommitSyntax {
        comment_prefix,
        trailer_separators: &options.trailer_separators,
        trailer_keys: &options.trailer_keys,
    };
    if git_trailer_lines(&before, commit) != git_trailer_lines(&after, commit) {
        return false;
    }
    // The words already match, so comparing the rendered HTML apart from whitespace catches
    // structural changes (a new thematic break, a quote nested into a list, a lost hard break).
    let render = |text: &str| {
        let mut html = String::new();
        pulldown_cmark::html::push_html(&mut html, Parser::new_ext(text, parser_options()));
        html
    };
    if !render(&before)
        .split_whitespace()
        .eq(render(&after).split_whitespace())
    {
        return false;
    }
    !check_idempotence
        || format_file_content_unchecked(formatted, Path::new(""), &RangeFilter::All, options)
            == formatted
}

fn format_file_content_unchecked(
    input: &str,
    canonical_path: &Path,
    range_filter: &RangeFilter,
    options: &FormattingOptions,
) -> String {
    if range_filter.is_filtered() && !range_filter.includes_file(canonical_path) {
        return input.to_string();
    }

    // A UTF-8 byte order mark (common in files saved on Windows) is not part of the Markdown:
    // format the text after it and put it back.
    if let Some(body) = input.strip_prefix('\u{feff}') {
        let mut out = String::from('\u{feff}');
        out.push_str(&format_file_content_unchecked(
            body,
            canonical_path,
            range_filter,
            options,
        ));
        return out;
    }

    let eol = options.line_ending.resolve(input);
    // Keep the file's last line as it was (ending in a newline or not) unless `.editorconfig`
    // says otherwise via `insert_final_newline`.
    let final_newline = options
        .insert_final_newline
        .unwrap_or_else(|| input.ends_with('\n'));

    if input.chars().all(|c| matches!(c, ' ' | '\t' | '\r' | '\n')) {
        return if final_newline && !input.is_empty() {
            eol.to_string()
        } else {
            String::new()
        };
    }

    let normalized = input.replace("\r\n", "\n");
    let resolved_opts;
    let comment_prefix = options
        .commit_message
        .then(|| preprocess::resolve_comment_prefix(&normalized, &options.comment_char))
        .filter(|prefix| *prefix != options.comment_char);
    let options = if let Some(prefix) = comment_prefix {
        let mut cloned = options.clone();
        cloned.comment_char = prefix.to_string();
        resolved_opts = cloned;
        &resolved_opts
    } else {
        options
    };
    let is_markdeep = options.markdeep.unwrap_or_else(|| {
        !options.commit_message && preprocess::has_markdeep_markers(&normalized)
    });

    let units = if options.commit_message {
        format_commit_message_units(&normalized, is_markdeep, options)
    } else {
        format_block_units(
            &normalized,
            1,
            options.max_line_width,
            true,
            is_markdeep,
            options,
            None,
        )
    };

    if !range_filter.is_filtered() {
        let mut out = render_root_units(&units, eol);
        if !final_newline && out.ends_with(eol) {
            out.truncate(out.len() - eol.len());
        }
        return out;
    }

    splice_filtered_units(input, eol, &units, canonical_path, range_filter)
}

fn render_root_units(units: &[BlockUnit], eol: &str) -> String {
    if units.is_empty() {
        return String::new();
    }
    let out_lines = join_block_unit_lines(units);
    let mut out = out_lines.join(eol);
    out.push_str(eol);
    out
}

fn join_block_unit_lines(units: &[BlockUnit]) -> Vec<String> {
    join_block_unit_lines_with_ranges(units).0
}

/// [`join_block_unit_lines`], also returning the output line indices of each unit.
fn join_block_unit_lines_with_ranges(units: &[BlockUnit]) -> (Vec<String>, Vec<Range<usize>>) {
    let mut out_lines = Vec::new();
    let mut ranges = Vec::with_capacity(units.len());
    for (idx, unit) in units.iter().enumerate() {
        if unit.lines.is_empty() {
            ranges.push(out_lines.len()..out_lines.len());
            continue;
        }
        if !out_lines.is_empty() {
            let prev = &units[idx - 1];
            let no_blank_between = prev.tight_after
                || unit.tight_before
                || (unit.is_list && unit.start_line == prev.end_line + 1);
            if !no_blank_between {
                out_lines.push(String::new());
            }
        }
        let start = out_lines.len();
        out_lines.extend(unit.lines.iter().cloned());
        ranges.push(start..out_lines.len());
    }
    (out_lines, ranges)
}

#[derive(Debug, Clone, Copy)]
struct OrigLine {
    content_start: usize,
    content_end: usize,
    full_end: usize,
}

fn split_orig_lines(input: &str) -> Vec<OrigLine> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut out = Vec::new();
    let mut pos = 0usize;

    while pos < len {
        let content_start = pos;
        while pos < len && bytes[pos] != b'\n' {
            pos += 1;
        }
        let mut content_end = pos;
        if content_end > content_start && bytes[content_end - 1] == b'\r' {
            content_end -= 1;
        }
        if pos < len && bytes[pos] == b'\n' {
            pos += 1;
        }
        out.push(OrigLine {
            content_start,
            content_end,
            full_end: pos,
        });
    }

    out
}

fn splice_filtered_units(
    input: &str,
    eol: &str,
    units: &[BlockUnit],
    canonical_path: &Path,
    range_filter: &RangeFilter,
) -> String {
    let orig_lines = split_orig_lines(input);
    if orig_lines.is_empty() {
        return input.to_string();
    }

    let overlapping: Vec<&BlockUnit> = units
        .iter()
        .filter(|u| {
            !u.lines.is_empty() && range_filter.overlaps(canonical_path, u.start_line, u.end_line)
        })
        .collect();

    if overlapping.is_empty() {
        return input.to_string();
    }

    let mut out = String::with_capacity(input.len());
    let mut next_orig_line = 1usize;

    for unit in overlapping {
        if unit.start_line == 0
            || unit.start_line > orig_lines.len()
            || unit.end_line < unit.start_line
            || unit.end_line > orig_lines.len()
            || unit.start_line < next_orig_line
        {
            continue;
        }

        let untouched_start_byte = orig_lines[next_orig_line - 1].content_start;
        let block_orig_start_byte = orig_lines[unit.start_line - 1].content_start;
        if untouched_start_byte < block_orig_start_byte {
            out.push_str(&input[untouched_start_byte..block_orig_start_byte]);
        }

        out.push_str(&unit.lines.join(eol));

        let last_orig = orig_lines[unit.end_line - 1];
        // Keep the original line ending, including none on the file's last line: partial
        // formatting never changes the end of the file.
        out.push_str(&input[last_orig.content_end..last_orig.full_end]);

        next_orig_line = unit.end_line + 1;
    }

    if next_orig_line <= orig_lines.len() {
        let tail_start = orig_lines[next_orig_line - 1].content_start;
        out.push_str(&input[tail_start..]);
    }

    out
}

fn format_commit_message_units(
    normalized: &str,
    is_markdeep: bool,
    options: &FormattingOptions,
) -> Vec<BlockUnit> {
    // `format_file_content` has already resolved `auto` to a concrete prefix.
    let comment_prefix = options.comment_char.as_str();
    let split = preprocess::split_commit_tail(normalized, comment_prefix);
    let head = &normalized[..split.head_end];

    // An empty or comment-only commit buffer (`git commit` aborted without typing a message) stays
    // byte-for-byte intact so Git still sees an empty message and aborts cleanly.
    if head.trim().is_empty() {
        let raw_trimmed = normalized.strip_suffix('\n').unwrap_or(normalized);
        let lines = raw_lines_verbatim(raw_trimmed);
        let end_line = lines.len().max(1);
        return vec![BlockUnit::verbatim(lines, 1, end_line)];
    }

    let mut units = format_block_units(
        head,
        1,
        options.max_line_width,
        true,
        is_markdeep,
        options,
        None,
    );
    preserve_units_creating_commit_syntax(head, &mut units, comment_prefix);
    let commit = preprocess::CommitSyntax {
        comment_prefix,
        trailer_separators: &options.trailer_separators,
        trailer_keys: &options.trailer_keys,
    };
    keep_blank_lines_before_frontmatter(head, &mut units, commit);

    let has_tail = split.tail_range.is_some();
    if let Some(tail_range) = split.tail_range {
        let tail_lines = raw_lines_verbatim(&normalized[tail_range]);
        // The head was trimmed before preprocessing, so its last standalone tag may appear
        // attached to whatever follows. The original separator takes precedence here.
        if split.had_blank_before_tail {
            if let Some(last) = units.last_mut() {
                last.tight_after = false;
            }
        }
        units.push(BlockUnit {
            tight_before: !split.had_blank_before_tail,
            ..BlockUnit::verbatim(tail_lines, split.tail_start_line, split.tail_end_line)
        });
    }
    // Include the tail: Git reads Jujutsu's `JJ:` comments as text or trailers.
    preserve_git_trailer_interpretation(normalized, &mut units, commit, has_tail);

    units
}

/// Ensures block normalization at root level (such as trimming indentation on rules, tables,
/// blockquotes, or standalone lines) never turns a non-comment line into a comment line,
/// introduces a `---` Git patch divider above the existing divider, or creates or removes a
/// `git am` patch break (`diff -`, `Index: `).
fn preserve_units_creating_commit_syntax(
    head: &str,
    units: &mut [BlockUnit],
    comment_prefix: &str,
) {
    let raw_trimmed = head.strip_suffix('\n').unwrap_or(head);
    let head_lines: Vec<&str> = raw_trimmed.split('\n').collect();
    let divider_line = head_lines
        .iter()
        .position(|l| preprocess::is_git_patch_divider(l))
        .map_or(usize::MAX, |idx| idx + 1);
    for unit in &mut *units {
        if unit.keeps_source_indent
            || unit.start_line == 0
            || unit.end_line < unit.start_line
            || unit.end_line > head_lines.len()
        {
            continue;
        }
        let orig = &head_lines[unit.start_line - 1..unit.end_line];
        let orig_comments = orig
            .iter()
            .filter(|l| preprocess::is_commit_comment_line(l, comment_prefix))
            .count();
        let new_comments = unit
            .lines
            .iter()
            .filter(|l| preprocess::is_commit_comment_line(l, comment_prefix))
            .count();
        let patch_breaks = |lines: &mut dyn Iterator<Item = &str>| {
            lines.filter(|l| preprocess::is_git_patch_break(l)).count()
        };
        let patch_break_changed = patch_breaks(&mut orig.iter().copied())
            != patch_breaks(&mut unit.lines.iter().map(String::as_str));
        let divider_changed = unit.end_line <= divider_line
            && orig.iter().any(|l| preprocess::is_git_patch_divider(l))
                != unit
                    .lines
                    .iter()
                    .any(|l| preprocess::is_git_patch_divider(l));
        if new_comments != orig_comments || divider_changed || patch_break_changed {
            *unit = BlockUnit {
                tight_before: unit.tight_before,
                tight_after: unit.tight_after,
                is_list: unit.is_list,
                ..BlockUnit::verbatim(
                    orig.iter().map(|s| (*s).to_string()).collect(),
                    unit.start_line,
                    unit.end_line,
                )
            };
        }
    }

    // Restoring or keeping a 2+-column-indented block after a list whose marker was narrowed
    // (for instance `-   item` -> `- item`, even across an attached standalone prompt tag) would
    // move that block inside the list item; keep the preceding list's source layout as well so
    // their relative indentation is preserved.
    preserve_lists_before_indented_verbatims(&head_lines, units, Some(comment_prefix));
}

fn preserve_lists_before_indented_verbatims(
    head_lines: &[&str],
    units: &mut [BlockUnit],
    comment_prefix: Option<&str>,
) {
    for i in (1..units.len()).rev() {
        let lead = units[i]
            .lines
            .first()
            .map_or(0, |l| preprocess::leading_ws_columns(l, TAB_STOP).0);
        if !units[i].keeps_source_indent || lead < 2 {
            continue;
        }
        let Some(prev_idx) = (0..i).rev().find(|&j| {
            let comment_only = comment_prefix.is_some_and(|prefix| {
                units[j]
                    .lines
                    .iter()
                    .all(|l| l.trim().is_empty() || preprocess::is_commit_comment_line(l, prefix))
            });
            let lazy_standalone = units[j].is_standalone && units[j].tight_before;
            !comment_only && !lazy_standalone
        }) else {
            continue;
        };
        let prev = &units[prev_idx];
        if prev.is_list
            && !prev.keeps_source_indent
            && prev.start_line > 0
            && prev.end_line >= prev.start_line
            && prev.end_line <= head_lines.len()
        {
            let orig = &head_lines[prev.start_line - 1..prev.end_line];
            units[prev_idx] = BlockUnit {
                tight_before: prev.tight_before,
                tight_after: prev.tight_after,
                is_list: true,
                ..BlockUnit::verbatim(
                    orig.iter().map(|s| (*s).to_string()).collect(),
                    prev.start_line,
                    prev.end_line,
                )
            };
        }
    }
}

/// Keeps the blank lines that open `head` when the formatted text after them would read as
/// frontmatter: without them, the next pass would format it differently (Git ignores leading
/// blank lines either way).
fn keep_blank_lines_before_frontmatter(
    head: &str,
    units: &mut Vec<BlockUnit>,
    commit: preprocess::CommitSyntax<'_>,
) {
    let blank_lines = head
        .split_inclusive('\n')
        .take_while(|line| line.trim().is_empty())
        .count();
    if blank_lines == 0
        || units.first().is_some_and(|u| u.start_line <= blank_lines)
        || !preprocess::starts_with_root_frontmatter(&render_root_units(units, "\n"), Some(commit))
    {
        return;
    }
    units.insert(
        0,
        BlockUnit {
            tight_after: true,
            ..BlockUnit::verbatim(vec![String::new(); blank_lines], 1, blank_lines)
        },
    );
}

/// The trailer lines Git parses from `text` (`git interpret-trailers --parse`, before
/// unfolding), or `None` if it finds no trailer block.
fn git_trailer_lines(text: &str, commit: preprocess::CommitSyntax<'_>) -> Option<Vec<String>> {
    let candidate =
        preprocess::find_git_trailer_candidate_in(text, commit).filter(|c| c.is_block)?;
    let lines: Vec<&str> = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .split('\n')
        .collect();
    Some(
        lines[candidate.lines]
            .iter()
            .filter(|l| !preprocess::is_git_comment_line(l, commit.comment_prefix))
            .map(|l| (*l).to_string())
            .collect(),
    )
}

/// Ensures formatting `message` leaves the trailers Git parses unchanged: reflowing or
/// normalizing blocks can join lines into a trailer (`https://...` joined onto one line), add
/// or remove the blank line that starts Git's trailer block (around a fence, or before the
/// message), or change what the block holds. The units Git reads differently are kept as
/// written, with their original blank-line separation; if that is not enough, so is the whole
/// message.
fn preserve_git_trailer_interpretation(
    message: &str,
    units: &mut Vec<BlockUnit>,
    commit: preprocess::CommitSyntax<'_>,
    has_tail: bool,
) {
    let expected = git_trailer_lines(message, commit);
    let render = |units: &[BlockUnit]| render_root_units(units, "\n");
    if git_trailer_lines(&render(units), commit) == expected {
        return;
    }
    let source_lines: Vec<&str> = message
        .strip_suffix('\n')
        .unwrap_or(message)
        .split('\n')
        .collect();
    let line_count = source_lines.len();
    let verbatim_lines = |start: usize, end: usize| -> BlockUnit {
        BlockUnit {
            tight_before: start > 1 && !source_lines[start - 2].trim().is_empty(),
            tight_after: end < line_count && !source_lines[end].trim().is_empty(),
            ..BlockUnit::verbatim(
                source_lines[start - 1..end]
                    .iter()
                    .map(|l| (*l).to_string())
                    .collect(),
                start,
                end,
            )
        }
    };
    let in_message = |u: &BlockUnit| u.start_line >= 1 && u.end_line <= line_count;

    // The units Git reads as (or around) the trailer block, before and after formatting.
    let mut picked: Vec<usize> = Vec::new();
    if let Some(orig) = preprocess::find_git_trailer_candidate_in(message, commit) {
        let (start, end) = (orig.lines.start + 1, orig.lines.end);
        picked.extend(
            (0..units.len()).filter(|&i| units[i].end_line >= start && units[i].start_line <= end),
        );
    }
    // Restoring just the units in the block Git finds after formatting usually suffices. If not,
    // the unit before it is restored too: the blank line between them is what Git reads. The
    // block starts right after that blank line, so the unit before it ends (exclusively) at
    // the blank line, `after.lines.start - 1`.
    let mut attempts = vec![picked.clone()];
    let rendered = render(units);
    if let Some(after) = preprocess::find_git_trailer_candidate_in(&rendered, commit) {
        let (_, ranges) = join_block_unit_lines_with_ranges(units);
        let overlapping = |after_start: usize| {
            (0..units.len())
                .filter(|&i| ranges[i].start < after.lines.end && ranges[i].end > after_start)
                .collect::<Vec<_>>()
        };
        attempts[0].extend(overlapping(after.lines.start.saturating_sub(1)));
        attempts.push(
            picked
                .iter()
                .copied()
                .chain(overlapping(after.lines.start.saturating_sub(2)))
                .collect(),
        );
    }
    for picked in attempts {
        let (Some(&first), Some(&last)) = (picked.iter().min(), picked.iter().max()) else {
            continue;
        };
        if !(first..=last).all(|i| in_message(&units[i])) {
            continue;
        }
        // Leading blank lines before the first unit can start Git's trailer search.
        let start = if first == 0 {
            1
        } else {
            units[first].start_line
        };
        let end = units[last].end_line.max(start);
        let mut candidate = units.clone();
        candidate.splice(first..=last, [verbatim_lines(start, end)]);
        if git_trailer_lines(&render(&candidate), commit) == expected {
            *units = candidate;
            return;
        }
    }
    // A detached comment/scissors tail is verbatim, including its trailing blank lines.
    let end = if has_tail {
        line_count
    } else {
        source_lines
            .iter()
            .rposition(|l| !l.trim().is_empty())
            .map_or(1, |i| i + 1)
    };
    *units = vec![verbatim_lines(1, end)];
}

fn format_block_units(
    text: &str,
    base_line: usize,
    width: usize,
    is_root: bool,
    is_markdeep: bool,
    options: &FormattingOptions,
    first_para_override: Option<FirstParaOverride<'_>>,
) -> Vec<BlockUnit> {
    let segments = preprocess::preprocess(text, is_root, is_markdeep, options.commit_syntax());
    let mut units: Vec<BlockUnit> = Vec::new();
    let mut para_override = first_para_override;

    for seg in segments {
        match seg {
            PreSegment::Frontmatter {
                range,
                start_line,
                end_line,
            } => {
                para_override = None;
                let lines = raw_lines_verbatim(&text[range]);
                units.push(BlockUnit::verbatim(
                    lines,
                    base_line + start_line - 1,
                    base_line + end_line - 1,
                ));
            }
            PreSegment::Verbatim {
                range,
                start_line,
                end_line,
                tight_before,
                tight_after,
            } => {
                para_override = None;
                let lines = raw_lines_verbatim(&text[range]);
                units.push(BlockUnit {
                    tight_before,
                    tight_after,
                    ..BlockUnit::verbatim(
                        lines,
                        base_line + start_line - 1,
                        base_line + end_line - 1,
                    )
                });
            }
            PreSegment::StandaloneLine {
                text: line_text,
                blank_before,
                blank_after,
                range: _,
                start_line,
                end_line,
            } => {
                para_override = None;
                units.push(BlockUnit {
                    tight_before: !blank_before,
                    tight_after: !blank_after,
                    is_standalone: true,
                    ..BlockUnit::new(
                        vec![line_text],
                        base_line + start_line - 1,
                        base_line + end_line - 1,
                    )
                });
            }
            PreSegment::Admonition {
                indent,
                header,
                body,
                range: _,
                start_line,
                end_line,
            } => {
                para_override = None;
                let lines = format_admonition(indent, &header, &body, width, is_markdeep, options);
                units.push(BlockUnit::new(
                    lines,
                    base_line + start_line - 1,
                    base_line + end_line - 1,
                ));
            }
            PreSegment::DefinitionList {
                term,
                blank_after_term,
                definitions,
                range: _,
                start_line,
                end_line,
            } => {
                para_override = None;
                let lines = format_definition_list(
                    &term,
                    blank_after_term,
                    &definitions,
                    width,
                    is_root,
                    is_markdeep,
                    options,
                );
                units.push(BlockUnit::new(
                    lines,
                    base_line + start_line - 1,
                    base_line + end_line - 1,
                ));
            }
            PreSegment::MarkdownChunk { range, start_line } => {
                let chunk = &text[range];
                let chunk_base_line = base_line + start_line - 1;
                let mut chunk_units = format_markdown_chunk(
                    chunk,
                    chunk_base_line,
                    width,
                    is_root,
                    is_markdeep,
                    options,
                    para_override.take(),
                );
                units.append(&mut chunk_units);
            }
        }
    }

    if is_root && !options.commit_message {
        let raw_trimmed = text.strip_suffix('\n').unwrap_or(text);
        let all_lines: Vec<&str> = raw_trimmed.split('\n').collect();
        preserve_lists_before_indented_verbatims(&all_lines, &mut units, None);
    }

    units
}

fn format_admonition(
    indent: usize,
    header: &str,
    body: &str,
    width: usize,
    is_markdeep: bool,
    options: &FormattingOptions,
) -> Vec<String> {
    let mut lines = Vec::new();
    let header_indent = " ".repeat(indent);
    lines.push(format!("{header_indent}{header}"));

    if body.trim().is_empty() {
        return lines;
    }

    let body_indent = indent + 4;
    let body_width = width.saturating_sub(body_indent).max(1);
    let inner_units = format_block_units(body, 1, body_width, false, is_markdeep, options, None);
    let inner_lines = join_block_unit_lines(&inner_units);
    let pad = " ".repeat(body_indent);
    for l in inner_lines {
        if l.is_empty() {
            lines.push(String::new());
        } else {
            lines.push(format!("{pad}{l}"));
        }
    }
    lines
}

/// Indentation for the lines of a container (list item, definition) after its first line.
#[derive(Debug, Clone, Copy)]
struct ContainerIndent {
    /// Continuation lines of the first paragraph.
    para: usize,
    /// Subsequent blocks (paragraphs, nested lists, fenced code, ...).
    block: usize,
    /// The container's content column (marker width); indented code blocks stay relative to it.
    content: usize,
}

impl ContainerIndent {
    /// An explicit `--hanging-indent` is used as given for paragraph continuation lines, while an
    /// `.editorconfig` `indent_size` only ever widens the indent. Block indentation is never less
    /// than the marker width (that would move the block out of the container), and no indent goes
    /// 4+ columns past the content column (that would turn nested blocks into indented code).
    fn for_marker(options: &FormattingOptions, marker_len: usize) -> Self {
        let max_indent = marker_len + 3;
        let minimum = options
            .min_hanging_indent
            .unwrap_or(0)
            .clamp(marker_len, max_indent);
        Self {
            para: options
                .hanging_indent
                .map_or(minimum, |h| h.min(max_indent)),
            block: options
                .hanging_indent
                .map_or(minimum, |h| h.clamp(marker_len, max_indent)),
            content: marker_len,
        }
    }
}

fn format_definition_list(
    term: &str,
    blank_after_term: bool,
    definitions: &[(String, String)],
    width: usize,
    is_root: bool,
    is_markdeep: bool,
    options: &FormattingOptions,
) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(term.to_string());
    if blank_after_term {
        lines.push(String::new());
    }

    for (marker_prefix, body) in definitions {
        let def_lines = format_prefixed_container(
            body,
            marker_prefix,
            ContainerIndent::for_marker(options, marker_prefix.len()),
            width,
            is_root,
            is_markdeep,
            options,
        );
        lines.extend(def_lines);
    }

    lines
}

fn format_markdown_chunk(
    chunk: &str,
    base_line: usize,
    width: usize,
    is_root: bool,
    is_markdeep: bool,
    options: &FormattingOptions,
    mut first_para_override: Option<FirstParaOverride<'_>>,
) -> Vec<BlockUnit> {
    let ast_items = collect_top_level_ast(chunk);
    let line_index = LineIndex::new(chunk, base_line);
    let mut units: Vec<BlockUnit> = Vec::new();
    let mut last_end = 0usize;

    for (index, original) in ast_items.iter().enumerate() {
        let mut item = original.clone();
        // Narrowing a list marker must not absorb a following top-level indented code block.
        if let TopAstItem::List {
            preserve_layout, ..
        } = &mut item
        {
            *preserve_layout = ast_items.get(index + 1).is_some_and(|next| {
                matches!(
                    next,
                    TopAstItem::CodeBlock {
                        kind_is_indented: true,
                        ..
                    }
                )
            });
        }
        let raw_range = item.range();
        let snapped_start = snap_to_line_start(chunk, raw_range.start, last_end);
        item.set_start(snapped_start);

        if snapped_start > last_end {
            let mut gap_units = emit_gap_blocks(chunk, last_end, snapped_start, &line_index);
            if !gap_units.is_empty() {
                first_para_override = None;
                units.append(&mut gap_units);
            }
        }

        let item_end = raw_range.end.min(chunk.len());
        let use_override = if first_para_override.is_some()
            && matches!(item, TopAstItem::Paragraph { .. })
            && chunk[..snapped_start]
                .bytes()
                .all(|b| b == b'\n' || b == b' ' || b == b'\t')
        {
            first_para_override.take()
        } else {
            first_para_override = None;
            None
        };

        let unit = format_top_ast_item(
            chunk,
            &item,
            &line_index,
            width,
            is_root,
            is_markdeep,
            options,
            use_override,
        );
        if !unit.lines.is_empty() {
            units.push(unit);
        }

        last_end = advance_past_eol(chunk, item_end);
    }

    if last_end < chunk.len() {
        let mut tail_units = emit_gap_blocks(chunk, last_end, chunk.len(), &line_index);
        units.append(&mut tail_units);
    }

    units
}

fn collect_top_level_ast(chunk: &str) -> Vec<TopAstItem> {
    let parser = Parser::new_ext(chunk, parser_options());
    let definitions: Vec<(Range<usize>, bool)> = parser
        .reference_definitions()
        .iter()
        .map(|(_, definition)| (definition.span.clone(), definition.title.is_some()))
        .collect();
    let parser = parser.into_offset_iter();
    let mut depth = 0usize;
    let mut top_start: Option<(Tag<'_>, usize)> = None;
    let mut list_items: Vec<Range<usize>> = Vec::new();
    let mut current_item_start: Option<usize> = None;
    let mut items = Vec::new();
    let mut has_empty_task = false;

    for (event, range) in parser {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    list_items.clear();
                    current_item_start = None;
                    top_start = Some((tag, range.start));
                } else if depth == 1 && matches!(tag, Tag::Item) {
                    current_item_start = Some(range.start);
                }
                depth += 1;
            }
            Event::End(tag_end) => {
                depth = depth.saturating_sub(1);
                if depth == 1 && matches!(tag_end, TagEnd::Item) {
                    if let Some(st) = current_item_start.take() {
                        list_items.push(st..range.end);
                    }
                } else if depth == 0 {
                    if let Some((start_tag, st)) = top_start.take() {
                        let span = st..range.end;
                        let item = match start_tag {
                            Tag::Paragraph => {
                                let follows_definition =
                                    definitions.iter().any(|(def_span, has_title)| {
                                        !has_title
                                            && def_span.end <= span.start
                                            && chunk[def_span.end..span.start].trim().is_empty()
                                            && chunk[def_span.end..span.start]
                                                .bytes()
                                                .filter(|&b| b == b'\n')
                                                .count()
                                                <= 1
                                    });
                                TopAstItem::Paragraph {
                                    range: span,
                                    follows_definition,
                                }
                            }
                            Tag::Heading { .. } => TopAstItem::Heading(span),
                            Tag::CodeBlock(kind) => TopAstItem::CodeBlock {
                                kind_is_indented: matches!(kind, CodeBlockKind::Indented),
                                range: span,
                            },
                            Tag::HtmlBlock => TopAstItem::HtmlBlock(span),
                            Tag::Table(_) => TopAstItem::Table(span),
                            Tag::BlockQuote(kind) => TopAstItem::BlockQuote {
                                is_alert: kind.is_some(),
                                range: span,
                            },
                            Tag::List(_) => TopAstItem::List {
                                range: span,
                                items: std::mem::take(&mut list_items),
                                preserve_layout: false,
                            },
                            Tag::FootnoteDefinition(label) => TopAstItem::Footnote {
                                label: label.to_string(),
                                range: span,
                            },
                            _ => TopAstItem::VerbatimBlock(span),
                        };
                        items.push(item);
                    }
                }
            }
            Event::Rule if depth == 0 => {
                items.push(TopAstItem::Rule(range));
            }
            Event::TaskListMarker(_) => {
                // Only a checkbox alone on its line and followed by another line (blank or lazy)
                // can end the list early; skip the second parse below for ordinary checklists.
                has_empty_task |=
                    chunk[range.end..]
                        .split_once('\n')
                        .is_some_and(|(line, after)| {
                            line.trim().is_empty() && !after.trim().is_empty()
                        });
            }
            _ => {}
        }
    }

    if has_empty_task {
        // An empty checkbox followed by a blank or lazy line can make pulldown-cmark end the
        // list earlier than CommonMark/GFM. Preserve the full ordinary list, including
        // its indented continuation, so neither interpretation changes on output.
        let mut options = parser_options();
        options.remove(pulldown_cmark::Options::ENABLE_TASKLISTS);
        let mut depth = 0usize;
        for (event, range) in Parser::new_ext(chunk, options).into_offset_iter() {
            match event {
                Event::Start(tag) => {
                    if depth == 0 && matches!(tag, Tag::List(_)) {
                        if let Some(index) = items.iter().position(|item| {
                            matches!(item, TopAstItem::List { range: original, .. }
                                if original.start == range.start && original.end < range.end)
                        }) {
                            let end = index
                                + items[index..]
                                    .iter()
                                    .take_while(|item| item.range().end <= range.end)
                                    .count();
                            if items
                                .get(end)
                                .is_none_or(|item| item.range().start >= range.end)
                            {
                                let source_end =
                                    trim_trailing_blank_end(chunk, range.start, range.end);
                                items.splice(
                                    index..end,
                                    [TopAstItem::VerbatimBlock(range.start..source_end)],
                                );
                            }
                        }
                    }
                    depth += 1;
                }
                Event::End(_) => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }

    items
}

fn snap_to_line_start(s: &str, pos: usize, min_pos: usize) -> usize {
    let clamped = pos.min(s.len()).max(min_pos);
    let mut cur = clamped;
    while cur > min_pos && s.as_bytes()[cur - 1] != b'\n' {
        cur -= 1;
    }
    cur
}

fn advance_past_eol(s: &str, pos: usize) -> usize {
    let mut p = pos.min(s.len());
    if p > 0 && p < s.len() && s.as_bytes()[p - 1] != b'\n' {
        // Container ranges can include the indentation of the next block, and footnotes can
        // share a line. Only consume the rest of a line when it contains no more content.
        let line_end = s[p..].find('\n').map_or(s.len(), |n| p + n);
        if !s[p..line_end].trim_matches([' ', '\t', '\r']).is_empty() {
            let start = s[..p].rfind('\n').map_or(0, |i| i + 1);
            return if s[start..p].trim_matches([' ', '\t']).is_empty() {
                start
            } else {
                p
            };
        }
        while p < s.len() && s.as_bytes()[p] != b'\n' {
            p += 1;
        }
        if p < s.len() && s.as_bytes()[p] == b'\n' {
            p += 1;
        }
    }
    p
}

fn emit_gap_blocks(
    chunk: &str,
    gap_start: usize,
    gap_end: usize,
    line_index: &LineIndex,
) -> Vec<BlockUnit> {
    let gap = &chunk[gap_start..gap_end];
    if gap.trim().is_empty() {
        return Vec::new();
    }

    let mut units = Vec::new();
    let mut group_lines: Vec<String> = Vec::new();
    let mut group_start_offset: Option<usize> = None;
    let mut group_end_offset = gap_start;

    let mut cursor = gap_start;
    for raw_line in gap.split('\n') {
        let next_cursor = (cursor + raw_line.len() + 1).min(gap_end);
        let is_blank = raw_line.trim().is_empty();
        if is_blank {
            if let Some(st) = group_start_offset.take() {
                units.push(BlockUnit::verbatim(
                    std::mem::take(&mut group_lines),
                    line_index.line_at(st),
                    line_index.line_at(group_end_offset),
                ));
            }
        } else {
            if group_start_offset.is_none() {
                group_start_offset = Some(cursor);
            }
            group_lines.push(raw_line.trim_end_matches('\r').to_string());
            group_end_offset = cursor;
        }
        cursor = next_cursor;
    }

    if let Some(st) = group_start_offset.take() {
        units.push(BlockUnit::verbatim(
            group_lines,
            line_index.line_at(st),
            line_index.line_at(group_end_offset),
        ));
    }

    if let Some(last) = units.last_mut() {
        last.tight_after = last.end_line + 1 == line_index.line_at(gap_end);
    }
    units
}

/// Maps byte offsets in a chunk to 1-based document line numbers in `O(log n)` per lookup.
struct LineIndex {
    newlines: Vec<usize>,
    base_line: usize,
}

impl LineIndex {
    fn new(s: &str, base_line: usize) -> Self {
        let newlines = s
            .bytes()
            .enumerate()
            .filter(|&(_, b)| b == b'\n')
            .map(|(i, _)| i)
            .collect();
        Self {
            newlines,
            base_line,
        }
    }

    /// Line number of `byte_offset`: `base_line` plus the number of newlines before it.
    fn line_at(&self, byte_offset: usize) -> usize {
        self.base_line + self.newlines.partition_point(|&nl| nl < byte_offset)
    }
}

fn trim_span_end(s: &str, start: usize, end: usize) -> usize {
    let mut e = end.min(s.len()).max(start);
    let bytes = s.as_bytes();
    while e > start && matches!(bytes[e - 1], b'\n' | b'\r') {
        e -= 1;
    }
    e
}

/// Chunks omit their final document newline. Interior parser spans include one separator
/// newline, which belongs to the block joiner; any other trailing newlines are literal content.
fn verbatim_span_end(chunk: &str, end: usize) -> usize {
    if end < chunk.len() && end > 0 && chunk.as_bytes()[end - 1] == b'\n' {
        end - 1
    } else {
        end
    }
}

fn trim_trailing_blank_end(s: &str, start: usize, end: usize) -> usize {
    let mut e = trim_span_end(s, start, end);
    while e > start {
        let mut line_start = e;
        while line_start > start && s.as_bytes()[line_start - 1] != b'\n' {
            line_start -= 1;
        }
        if s[line_start..e].trim().is_empty() {
            e = trim_span_end(s, start, line_start);
        } else {
            break;
        }
    }
    e
}

#[allow(clippy::too_many_arguments)]
fn format_top_ast_item(
    chunk: &str,
    item: &TopAstItem,
    line_index: &LineIndex,
    width: usize,
    is_root: bool,
    is_markdeep: bool,
    options: &FormattingOptions,
    first_para_override: Option<FirstParaOverride<'_>>,
) -> BlockUnit {
    let range = item.range();
    let trimmed_end = trim_trailing_blank_end(chunk, range.start, range.end);
    let start_line = line_index.line_at(range.start);
    let end_line = if trimmed_end > range.start {
        line_index.line_at(trimmed_end - 1)
    } else {
        start_line
    };
    let raw_slice = &chunk[range.start..trimmed_end];

    match item {
        TopAstItem::Paragraph {
            follows_definition, ..
        } => {
            // Some paragraphs depend on preceding reference definitions: parsed alone, their
            // first line would be a list or code block. Retain that source indentation/context.
            if !matches!(
                Parser::new_ext(raw_slice, parser_options()).next(),
                Some(Event::Start(Tag::Paragraph))
            ) {
                return BlockUnit::verbatim(raw_lines_verbatim(raw_slice), start_line, end_line);
            }
            let lines = format_paragraph_lines(
                raw_slice,
                width,
                is_root,
                options,
                first_para_override,
                *follows_definition,
            );
            BlockUnit {
                is_paragraph: true,
                ..BlockUnit::new(lines, start_line, end_line)
            }
        }
        TopAstItem::Heading(_) => {
            let root_commit_prefix = options
                .commit_message
                .then_some(options.comment_char.as_str())
                .filter(|_| is_root);
            let lines = format_heading_lines(raw_slice, options.commit_message, root_commit_prefix);
            BlockUnit::new(lines, start_line, end_line)
        }
        TopAstItem::CodeBlock {
            kind_is_indented, ..
        } => {
            let raw_end = if *kind_is_indented {
                trimmed_end
            } else {
                verbatim_span_end(chunk, range.end)
            };
            let end_l = if raw_end > range.start {
                line_index.line_at(raw_end - 1)
            } else {
                start_line
            };
            let lines = raw_lines_verbatim(&chunk[range.start..raw_end]);
            BlockUnit::verbatim(lines, start_line, end_l)
        }
        TopAstItem::HtmlBlock(_) | TopAstItem::VerbatimBlock(_) => {
            let end = verbatim_span_end(chunk, range.end);
            let lines = raw_lines_verbatim(&chunk[range.start..end]);
            BlockUnit::verbatim(lines, start_line, line_index.line_at(end.saturating_sub(1)))
        }
        TopAstItem::Rule(_) => {
            let root_commit_prefix = options
                .commit_message
                .then_some(options.comment_char.as_str())
                .filter(|_| is_root);
            let orig_has_divider = raw_slice.lines().any(preprocess::is_git_patch_divider);
            let orig_has_comment = root_commit_prefix.is_some_and(|prefix| {
                raw_slice
                    .lines()
                    .any(|l| preprocess::is_commit_comment_line(l, prefix))
            });
            let trimmed_lines: Vec<String> = raw_slice
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .map(|mut l| {
                    let creates_divider = root_commit_prefix.is_some()
                        && !orig_has_divider
                        && preprocess::is_git_patch_divider(&l);
                    let creates_comment = root_commit_prefix.is_some_and(|prefix| {
                        !orig_has_comment && preprocess::is_commit_comment_line(&l, prefix)
                    });
                    if creates_divider || creates_comment {
                        l.insert(0, ' ');
                    }
                    l
                })
                .collect();
            BlockUnit::new(trimmed_lines, start_line, end_line)
        }
        TopAstItem::Table(_) => {
            let mut lines = if options.align_table_columns {
                table::format_table(raw_slice, "")
                    .unwrap_or_else(|| raw_lines_trimmed_end(raw_slice))
            } else {
                raw_lines_trimmed_end(raw_slice)
            };
            // Table alignment can resize delimiter runs, add outer pipes, or fill missing
            // cells. Commit messages permit only whitespace changes, so keep the source when
            // the aligned table would change its words.
            if options.commit_message
                && !raw_slice
                    .split_whitespace()
                    .eq(lines.iter().flat_map(|line| line.split_whitespace()))
            {
                lines = raw_lines_trimmed_end(raw_slice);
            }
            BlockUnit::new(lines, start_line, end_line)
        }
        TopAstItem::BlockQuote { is_alert, .. } => {
            // In a commit message, which is read as plain text, reflowing a quote adds or drops
            // `>` markers (lazy continuation lines join the quote), so quotes stay as written.
            let lines = if options.commit_message {
                // Drop only the quote's own indentation (at most 3 spaces), which can move it into
                // a preceding list once that list's markers are normalized.
                let lines = raw_lines_verbatim(raw_slice);
                let indent = lines
                    .first()
                    .map_or(0, |line| line.len() - line.trim_start_matches(' ').len())
                    .min(3);
                lines
                    .into_iter()
                    .map(|line| {
                        let spaces = line.len() - line.trim_start_matches(' ').len();
                        line[spaces.min(indent)..].to_string()
                    })
                    .collect()
            } else {
                format_blockquote_lines(raw_slice, *is_alert, width, is_markdeep, options)
            };
            BlockUnit::new(lines, start_line, end_line)
        }
        TopAstItem::List {
            items: item_ranges,
            preserve_layout,
            ..
        } => {
            let lines = format_list_lines(
                chunk,
                range.clone(),
                item_ranges,
                width,
                is_root,
                is_markdeep,
                options,
                *preserve_layout,
            );
            let trailing_definition = item_ranges
                .last()
                .is_some_and(|item| !chunk[item.end..range.end].trim().is_empty());
            BlockUnit {
                is_list: true,
                tight_after: trailing_definition && line_index.line_at(range.end) == end_line + 1,
                ..BlockUnit::new(lines, start_line, end_line)
            }
        }
        TopAstItem::Footnote { label, .. } => {
            let lines = format_footnote_lines(raw_slice, label, width, is_markdeep, options);
            BlockUnit::new(lines, start_line, end_line)
        }
    }
}

fn raw_lines_verbatim(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split('\n').map(|s| s.to_string()).collect()
}

fn raw_lines_trimmed_end(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split('\n')
        .map(|s| s.trim_end_matches([' ', '\t', '\r']).to_string())
        .collect()
}

fn format_paragraph_lines(
    raw: &str,
    width: usize,
    is_root: bool,
    options: &FormattingOptions,
    first_para_override: Option<FirstParaOverride<'_>>,
    protect_first_line: bool,
) -> Vec<String> {
    let commit_comment = options.commit_syntax();
    let commit_level = if is_root {
        inline::CommitLevel::Root
    } else if first_para_override.is_some_and(|ov| ov.root_continuations) {
        inline::CommitLevel::RootContinuation
    } else {
        inline::CommitLevel::Nested
    };
    let protect_root = commit_level != inline::CommitLevel::Nested;
    let segments = inline::tokenize_paragraph(
        raw,
        options.wrap,
        protect_first_line,
        commit_comment,
        commit_level,
    );
    let mut out_lines: Vec<String> = Vec::new();
    let source_indents = if options.wrap == crate::options::WrapMode::Preserve {
        inline::preserved_line_indents(raw)
    } else {
        Vec::new()
    };
    let mut source_line = 0usize;

    for (seg_idx, segment) in segments.iter().enumerate() {
        let tokens = &segment.tokens;
        let (first_w, rest_w, line0_prefix, cont_prefix) = match first_para_override {
            Some(ov) if seg_idx == 0 => (
                ov.first_width,
                ov.rest_width,
                ov.first_prefix,
                " ".repeat(ov.rest_indent),
            ),
            Some(ov) => (ov.rest_width, ov.rest_width, "", " ".repeat(ov.rest_indent)),
            None => (width, width, "", String::new()),
        };

        let seg_lines =
            reflow::reflow_segment(tokens, first_w, rest_w, options.wrap, options.optimal);
        let mut seg_lines: Vec<String> = seg_lines
            .into_iter()
            .flat_map(|line| line.split('\n').map(str::to_string).collect::<Vec<_>>())
            .collect();
        if seg_lines.is_empty() {
            seg_lines.push(String::new());
        }

        let seg_source_line = source_line;
        if options.wrap == crate::options::WrapMode::Preserve {
            for line in &mut seg_lines {
                if let Some(indent) = source_indents.get(source_line) {
                    line.insert_str(0, indent);
                }
                source_line += 1;
            }
        }

        if let Some(ov) = first_para_override {
            if seg_idx == 0 {
                if seg_lines[0].is_empty() {
                    seg_lines[0] = line0_prefix.trim_end().to_string();
                } else {
                    seg_lines[0] = format!("{line0_prefix}{}", seg_lines[0]);
                }
                for l in seg_lines.iter_mut().skip(1) {
                    if !l.is_empty() {
                        *l = format!("{cont_prefix}{l}");
                    }
                }
            } else {
                let pad = " ".repeat(ov.rest_indent);
                for l in seg_lines.iter_mut() {
                    if !l.is_empty() {
                        *l = format!("{pad}{l}");
                    }
                }
            }
        }

        if options.wrap == crate::options::WrapMode::Preserve
            && protect_root
            && options.commit_message
        {
            let skip = usize::from(first_para_override.is_some() && seg_idx == 0);
            // Preserve mode keeps one output line per source line. Collapsing whitespace (such
            // as `diff  -u` or `Index:\tfoo`) must not create a `git am` patch break either.
            let raw_lines: Vec<&str> = raw.split('\n').collect();
            for (k, line) in seg_lines.iter_mut().enumerate().skip(skip) {
                let created_patch_break = preprocess::is_git_patch_break(line)
                    && !raw_lines
                        .get(seg_source_line + k)
                        .is_some_and(|orig| preprocess::is_git_patch_break(orig));
                if preprocess::is_commit_comment_line(line, &options.comment_char)
                    || created_patch_break
                {
                    line.insert(0, ' ');
                }
            }
        }

        if let Some(hard_break) = segment.hard_break {
            if let Some(last) = seg_lines.last_mut() {
                last.push_str(hard_break.marker());
            }
        }

        out_lines.extend(seg_lines);
    }

    out_lines
}

fn format_heading_lines(
    raw: &str,
    commit_message: bool,
    root_commit_prefix: Option<&str>,
) -> Vec<String> {
    let raw_lines: Vec<&str> = raw.split('\n').collect();
    if raw_lines.len() >= 2 {
        let last_trimmed = raw_lines[raw_lines.len() - 1].trim();
        let is_setext_underline = !last_trimmed.is_empty()
            && (last_trimmed.bytes().all(|b| b == b'=') || last_trimmed.bytes().all(|b| b == b'-'));
        if is_setext_underline {
            // Multiline headings can contain significant indentation, hard breaks, and
            // inline literals crossing lines. Preserve that body and normalize its underline.
            let mut content_lines: Vec<String> = if raw_lines.len() == 2 {
                let mut line0 = collapse_inline_spaces(raw_lines[0]);
                if root_commit_prefix.is_some_and(|prefix| {
                    preprocess::is_commit_comment_line(&line0, prefix)
                        || (preprocess::is_git_patch_break(&line0)
                            && !preprocess::is_git_patch_break(raw_lines[0]))
                }) {
                    line0.insert(0, ' ');
                }
                vec![line0]
            } else {
                raw_lines[..raw_lines.len() - 1]
                    .iter()
                    .map(|line| line.to_string())
                    .collect()
            };
            let max_w = content_lines
                .iter()
                .map(|line| UnicodeWidthStr::width(line.as_str()))
                .max()
                .unwrap_or(0);
            let raw_underline = raw_lines[raw_lines.len() - 1];
            let mut underline: String = if commit_message {
                // Git reads even a nested heading's underline as a word of the message.
                last_trimmed.to_string()
            } else {
                let underline_ch = last_trimmed.chars().next().unwrap_or('=');
                std::iter::repeat_n(underline_ch, max_w.max(3)).collect()
            };
            // Dedenting a root underline must not create a `---` patch divider.
            if root_commit_prefix.is_some()
                && preprocess::is_git_patch_divider(&underline)
                && !preprocess::is_git_patch_divider(raw_underline)
            {
                underline.insert(0, ' ');
            }
            if root_commit_prefix
                .is_some_and(|prefix| preprocess::is_commit_comment_line(&underline, prefix))
            {
                underline.insert(0, ' ');
            }
            content_lines.push(underline);
            return content_lines;
        }
    }

    // ATX heading (single line)
    let line = raw_lines
        .first()
        .copied()
        .unwrap_or("")
        .trim_start_matches(' ')
        .trim_end_matches([' ', '\t', '\r']);
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return vec![line.to_string()];
    }
    let prefix = &line[..hashes];
    let after = line[hashes..].trim_start_matches([' ', '\t']);
    let mut formatted = if after.is_empty() {
        prefix.to_string()
    } else {
        // Preserve optional closing `#` sequence if present per CommonMark (preceded by horizontal space).
        let trimmed_end = after.trim_end_matches('#');
        let closing_hashes = &after[trimmed_end.len()..];
        if !closing_hashes.is_empty()
            && (trimmed_end.is_empty() || trimmed_end.ends_with([' ', '\t']))
        {
            let body = collapse_inline_spaces(trimmed_end);
            if body.is_empty() {
                format!("{prefix} {closing_hashes}")
            } else {
                format!("{prefix} {body} {closing_hashes}")
            }
        } else {
            let body = collapse_inline_spaces(after);
            if body.is_empty() {
                prefix.to_string()
            } else {
                format!("{prefix} {body}")
            }
        }
    };
    if root_commit_prefix
        .is_some_and(|prefix| preprocess::is_commit_comment_line(&formatted, prefix))
    {
        formatted.insert(0, ' ');
    }
    vec![formatted]
}

/// Normalize prose spacing while keeping code, link destinations/titles, and HTML attributes.
fn collapse_inline_spaces(s: &str) -> String {
    inline::tokenize_paragraph(
        s,
        crate::options::WrapMode::Never,
        false,
        None,
        inline::CommitLevel::Nested,
    )
    .iter()
    .flat_map(|segment| segment.tokens.iter().map(|token| token.text.as_str()))
    .collect::<Vec<_>>()
    .join(" ")
}

fn strip_blockquote_prefix(line: &str, tab_width: usize) -> String {
    let tw = tab_width.max(1);
    let bytes = line.as_bytes();
    let mut cols = 0usize;
    let mut i = 0usize;

    while i < bytes.len() && cols <= 3 {
        match bytes[i] {
            b' ' => {
                cols += 1;
                i += 1;
            }
            b'\t' => {
                let adv = tw - (cols % tw);
                if cols + adv > 3 {
                    return line.trim_start().to_string();
                }
                cols += adv;
                i += 1;
            }
            _ => break,
        }
    }

    if i < bytes.len() && bytes[i] == b'>' {
        i += 1;
        cols += 1;
        if i < bytes.len() {
            if bytes[i] == b' ' {
                i += 1;
            } else if bytes[i] == b'\t' {
                let adv = tw - (cols % tw);
                let remaining = adv.saturating_sub(1);
                let mut out = " ".repeat(remaining);
                out.push_str(&line[i + 1..]);
                return out;
            }
        }
        line[i..].to_string()
    } else {
        // Lazy continuation line inside blockquote
        line.trim_start().to_string()
    }
}

fn is_github_alert_marker(trimmed: &str) -> bool {
    matches!(
        trimmed.to_ascii_uppercase().as_str(),
        "[!NOTE]" | "[!TIP]" | "[!IMPORTANT]" | "[!WARNING]" | "[!CAUTION]"
    )
}

fn format_blockquote_lines(
    raw: &str,
    is_alert: bool,
    width: usize,
    is_markdeep: bool,
    options: &FormattingOptions,
) -> Vec<String> {
    let mut inner_lines: Vec<String> = Vec::new();
    let breaks = soft_breaks(raw);
    let mut offset = 0usize;
    for line in raw.split('\n') {
        let lazy_continuation = follows_soft_break(&breaks, offset);
        offset += line.len() + 1;
        let explicit = line.trim_start_matches(' ').starts_with('>');
        if !explicit && options.wrap == crate::options::WrapMode::Preserve {
            // Adding a `>` to a lazy continuation can introduce a new block. Keeping its
            // existing source boundary is the preserve mode's contract.
            return raw_lines_verbatim(raw);
        }
        let content = strip_blockquote_prefix(line, TAB_STOP);
        if !explicit && lazy_continuation {
            // Lazy continuations belong to the already-open paragraph, even when their text
            // resembles a setext underline or list marker after stripping the container.
            if let Some(previous) = inner_lines.last_mut() {
                previous.push(' ');
                previous.push_str(&content);
            }
        } else {
            inner_lines.push(content);
        }
    }

    if inner_lines.is_empty() {
        return vec![">".to_string()];
    }

    let inner_width = width.saturating_sub(2).max(1);
    let first_trimmed = inner_lines[0].trim();

    // A marker the parser did not accept as an alert is literal text: keep that line exactly as
    // written (a different indentation could turn it into an alert). Four or more columns make
    // it indented code, which the general path below preserves. Check past leading blank quote
    // lines (`>\n> [!NOTE]`) as well, since stripping those blanks would promote it to line 0.
    let first_non_blank = inner_lines
        .iter()
        .find(|line| !line.trim().is_empty())
        .map_or("", String::as_str);
    let first_indent = preprocess::leading_ws_columns(first_non_blank, TAB_STOP).0;
    let literal_marker = !is_alert
        && first_indent < 4
        && first_non_blank
            .trim()
            .split_once(']')
            .is_some_and(|(marker, _)| is_github_alert_marker(&format!("{marker}]")));
    if literal_marker {
        // Reflow could isolate the marker, remove a hard break, or reinterpret the following
        // line as a new block. Keep the original quote when its alert-like prefix is literal.
        return raw_lines_verbatim(raw);
    }
    if is_alert {
        let mut out = vec![format!("> {first_trimmed}")];
        if inner_lines.len() > 1 {
            let had_blank = inner_lines[1].trim().is_empty();
            if !had_blank && inner_lines[0].ends_with("  ") {
                // Renderers without alert support read the marker line as text, where the
                // trailing spaces are a hard break before the body.
                out[0].push_str("  ");
            }
            let rest_str = inner_lines[1..].join("\n");
            let units =
                format_block_units(&rest_str, 1, inner_width, false, is_markdeep, options, None);
            let formatted_rest = join_block_unit_lines(&units);
            if !formatted_rest.is_empty() {
                if had_blank {
                    out.push(">".to_string());
                }
                for l in formatted_rest {
                    if l.is_empty() {
                        out.push(">".to_string());
                    } else {
                        out.push(format!("> {l}"));
                    }
                }
            }
        }
        return out;
    }

    let inner_str = inner_lines.join("\n");
    let units = format_block_units(
        &inner_str,
        1,
        inner_width,
        false,
        is_markdeep,
        options,
        None,
    );
    let formatted_inner = join_block_unit_lines(&units);
    if formatted_inner.is_empty() {
        return vec![">".to_string()];
    }
    formatted_inner
        .into_iter()
        .map(|l| {
            if l.is_empty() {
                ">".to_string()
            } else {
                format!("> {l}")
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn format_list_lines(
    chunk: &str,
    list_range: Range<usize>,
    item_ranges: &[Range<usize>],
    width: usize,
    is_root: bool,
    is_markdeep: bool,
    options: &FormattingOptions,
    preserve_layout: bool,
) -> Vec<String> {
    if item_ranges.is_empty() {
        return Vec::new();
    }

    let mut clean_spans: Vec<Range<usize>> = Vec::with_capacity(item_ranges.len());
    let mut prev_end = list_range.start;
    for r in item_ranges {
        let start = snap_to_line_start(chunk, r.start, prev_end);
        let end = trim_trailing_blank_end(chunk, start, r.end);
        clean_spans.push(start..end);
        prev_end = r.end.max(end);
    }

    let preserve_layout = preserve_layout
        || (options.wrap == crate::options::WrapMode::Preserve
            && clean_spans.iter().any(|span| {
                let raw = &chunk[span.clone()];
                let first = raw.lines().next().unwrap_or("");
                let (_, column, _) = parse_list_marker(first, TAB_STOP);
                let breaks = soft_breaks(raw);
                let mut offset = first.len() + 1;
                raw.split('\n').skip(1).any(|line| {
                    let lazy = preprocess::leading_ws_columns(line, TAB_STOP).0 < column
                        && follows_soft_break(&breaks, offset);
                    offset += line.len() + 1;
                    lazy
                })
            }));
    let numbering = ordered_list_numbering(chunk, &clean_spans, options);
    let mut out_lines: Vec<String> = Vec::with_capacity(clean_spans.len());
    for (idx, span) in clean_spans.iter().enumerate() {
        let raw_item = &chunk[span.start..span.end];
        let marker_override = numbering.as_ref().and_then(|n| n.marker_for(idx));
        let item_lines = format_single_list_item(
            raw_item,
            marker_override,
            width,
            is_root,
            is_markdeep,
            options,
            preserve_layout,
        );
        out_lines.extend(item_lines);
        if idx + 1 == clean_spans.len() {
            let list_end = list_range.end;
            let gap_start = advance_past_eol(chunk, item_ranges[idx].end);
            if gap_start < list_end && !chunk[gap_start..list_end].trim().is_empty() {
                out_lines.push(String::new());
                out_lines.extend(raw_lines_verbatim(
                    chunk[gap_start..list_end].trim_end_matches([' ', '\t', '\n']),
                ));
            }
        }

        if idx + 1 < clean_spans.len() {
            let next_start = clean_spans[idx + 1].start;
            let between = &chunk[span.end..next_start];
            let has_blank_line = between.bytes().filter(|&b| b == b'\n').count() >= 2;
            if has_blank_line {
                out_lines.push(String::new());
            }
        }
    }

    out_lines
}

/// Renumbering for an ordered list under a non-`Preserve` [`OrderedListStyle`].
struct OrderedNumbering {
    start: u64,
    delimiter: char,
    style: OrderedListStyle,
}

impl OrderedNumbering {
    /// The marker for item `idx`, or `None` to keep it as written (the first item always keeps
    /// its marker, since its number sets the list start).
    fn marker_for(&self, idx: usize) -> Option<String> {
        if idx == 0 {
            return None;
        }
        let number = match self.style {
            OrderedListStyle::Preserve => return None,
            OrderedListStyle::Ascending => self.start.checked_add(idx as u64)?,
            OrderedListStyle::One => self.start,
        };
        // CommonMark ordered list markers allow at most 9 digits.
        (number <= 999_999_999).then(|| format!("{number}{}", self.delimiter))
    }
}

fn ordered_list_numbering(
    chunk: &str,
    item_spans: &[Range<usize>],
    options: &FormattingOptions,
) -> Option<OrderedNumbering> {
    if options.ordered_lists == OrderedListStyle::Preserve {
        return None;
    }
    let first = item_spans.first()?;
    let first_line = chunk[first.clone()].split('\n').next()?;
    let (marker, _, _) = parse_list_marker(first_line, TAB_STOP);
    let delimiter = marker.chars().last().filter(|c| matches!(c, '.' | ')'))?;
    let start = marker[..marker.len() - 1].parse().ok()?;
    Some(OrderedNumbering {
        start,
        delimiter,
        style: options.ordered_lists,
    })
}

fn format_single_list_item(
    raw_item: &str,
    marker_override: Option<String>,
    width: usize,
    is_root: bool,
    is_markdeep: bool,
    options: &FormattingOptions,
    preserve_layout: bool,
) -> Vec<String> {
    let raw_lines: Vec<&str> = raw_item.split('\n').collect();
    let Some(&line0) = raw_lines.first() else {
        return Vec::new();
    };

    let (marker, strip_cols, rem0) = parse_list_marker(line0, TAB_STOP);
    let breaks = soft_breaks(raw_item);
    let mut line_offset = line0.len() + 1;
    let lazy_lines: Vec<bool> = raw_lines[1..]
        .iter()
        .map(|line| {
            let lazy = preprocess::leading_ws_columns(line, TAB_STOP).0 < strip_cols
                && follows_soft_break(&breaks, line_offset);
            line_offset += line.len() + 1;
            lazy
        })
        .collect();
    let has_lazy = lazy_lines.contains(&true);
    // Renderers disagree about a checkbox that ends its line (`- [ ]`) followed by a lazy line:
    // pulldown-cmark 0.12 reads the checkbox as text, 0.13 ends the list after it, and GFM keeps
    // the lazy text in the item. Joining or indenting that line (which could also turn it into a
    // setext underline) would pick one reading, so keep the layout.
    let bare_checkbox_before_lazy =
        lazy_lines.first() == Some(&true) && matches!(rem0.as_str(), "[ ]" | "[x]" | "[X]");
    let preserve_lazy = has_lazy
        && (options.wrap == crate::options::WrapMode::Preserve || bare_checkbox_before_lazy);
    let marker = marker_override.unwrap_or(marker);
    let marker_prefix = if preserve_layout || preserve_lazy {
        let lead = preprocess::leading_ws_columns(line0, TAB_STOP).0;
        format!(
            "{}{marker}{}",
            " ".repeat(lead),
            " ".repeat(strip_cols.saturating_sub(lead + marker.len()).max(1))
        )
    } else {
        format!("{marker} ")
    };

    if preserve_lazy {
        // A lazy line's indentation can be the reason its apparent marker remains text. Keep
        // that layout in preserve mode, shifting only when renumbering widens the marker.
        let extra = marker_prefix.len().saturating_sub(strip_cols);
        let mut lines = vec![format!("{marker_prefix}{rem0}")];
        lines.extend(raw_lines[1..].iter().map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{}{line}", " ".repeat(extra))
            }
        }));
        return lines;
    }

    let (task_prefix, after_task) = extract_task_checkbox(&rem0);
    let mut body_lines: Vec<String> = Vec::with_capacity(raw_lines.len());
    body_lines.push(after_task.to_string());
    for (&cont, &lazy) in raw_lines[1..].iter().zip(&lazy_lines) {
        if lazy {
            let previous = body_lines.last_mut().unwrap();
            previous.push(' ');
            previous.push_str(cont.trim_start_matches([' ', '\t']));
            continue;
        }
        if cont.trim().is_empty() {
            body_lines.push(String::new());
        } else {
            body_lines.push(dedent_columns(cont, strip_cols, TAB_STOP));
        }
    }
    let body = body_lines.join("\n");
    let source_spaced = rem0.len() > rem0.trim_end_matches([' ', '\t']).len();
    let render_with_marker = |marker_prefix: &str| {
        let first_prefix = format!("{marker_prefix}{task_prefix}");
        if body.trim().is_empty() {
            // Whether whitespace follows an otherwise empty checkbox has decided between a task item
            // (`- [ ] `) and literal text (`- [ ]`) in some renderers, so keep what the source had.
            return vec![if task_prefix.is_empty() || !source_spaced {
                first_prefix.trim_end().to_string()
            } else {
                first_prefix
            }];
        }

        let mut lines = format_prefixed_container(
            &body,
            &first_prefix,
            ContainerIndent::for_marker(options, marker_prefix.len()),
            width,
            is_root,
            is_markdeep,
            options,
        );
        if !task_prefix.is_empty() && after_task.is_empty() && lines.len() > 1 {
            // A checkbox alone on the marker line needs its trailing whitespace for task
            // recognition. Also retain any blank line separating it from the next block.
            if source_spaced {
                lines[0] = first_prefix;
            }
            if raw_lines.get(1).is_some_and(|line| line.trim().is_empty()) && !lines[1].is_empty() {
                lines.insert(1, String::new());
            }
        }
        lines
    };

    let lines = render_with_marker(&marker_prefix);
    if is_root
        && options.commit_message
        && !preserve_layout
        && lines
            .first()
            .is_some_and(|l| preprocess::is_commit_comment_line(l, &options.comment_char))
    {
        return render_with_marker(&format!(" {marker} "));
    }
    lines
}

fn parse_list_marker(line: &str, tab_width: usize) -> (String, usize, String) {
    let tw = tab_width.max(1);
    let bytes = line.as_bytes();
    let mut cols = 0usize;
    let mut i = 0usize;

    while i < bytes.len() && cols <= 3 {
        match bytes[i] {
            b' ' => {
                cols += 1;
                i += 1;
            }
            b'\t' => {
                cols += tw - (cols % tw);
                i += 1;
            }
            _ => break,
        }
    }

    if i >= bytes.len() {
        return ("-".to_string(), cols + 2, String::new());
    }

    let marker_start = i;
    if matches!(bytes[i], b'-' | b'*' | b'+') {
        i += 1;
    } else {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i < bytes.len() && matches!(bytes[i], b'.' | b')') {
            i += 1;
        }
    }

    let marker = &line[marker_start..i];
    let after_marker_cols = cols + marker.len();

    // Measure whitespace columns immediately after marker
    let ws_start = i;
    let mut ws_cols = 0usize;
    let mut cur_col = after_marker_cols;
    while i < bytes.len() {
        match bytes[i] {
            b' ' => {
                ws_cols += 1;
                cur_col += 1;
                i += 1;
            }
            b'\t' => {
                let adv = tw - (cur_col % tw);
                ws_cols += adv;
                cur_col += adv;
                i += 1;
            }
            _ => break,
        }
    }

    let rest = &line[i..];
    if rest.is_empty() {
        return (marker.to_string(), after_marker_cols + 1, String::new());
    }

    if ws_cols <= 4 {
        (
            marker.to_string(),
            after_marker_cols + ws_cols.max(1),
            rest.to_string(),
        )
    } else {
        // CommonMark indented code on item line 0 (`-     code`): consume only 1 space after marker.
        let strip_after_marker = &line[ws_start..];
        let trimmed_one = dedent_columns(strip_after_marker, 1, tw);
        (marker.to_string(), after_marker_cols + 1, trimmed_one)
    }
}

fn extract_task_checkbox(rem: &str) -> (&'static str, &str) {
    for (checkbox, prefix) in [("[ ]", "[ ] "), ("[x]", "[x] "), ("[X]", "[X] ")] {
        if let Some(rest) = rem.strip_prefix(checkbox) {
            if rest.is_empty() || rest.starts_with([' ', '\t']) {
                return (prefix, rest.trim_start_matches([' ', '\t']));
            }
        }
    }
    ("", rem)
}

/// The `[^label]:` marker at the start of `line`, exactly as written.
fn raw_footnote_marker(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("[^")?;
    let mut escaped = false;
    for (i, ch) in rest.char_indices() {
        match ch {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            ']' => {
                return rest[i + 1..].starts_with(':').then(|| &line[..2 + i + 2]);
            }
            '[' => return None,
            _ => {}
        }
    }
    None
}

fn format_footnote_lines(
    raw: &str,
    label: &str,
    width: usize,
    is_markdeep: bool,
    options: &FormattingOptions,
) -> Vec<String> {
    let raw_lines: Vec<&str> = raw.split('\n').collect();
    let Some(&line0) = raw_lines.first() else {
        return Vec::new();
    };

    // The parser normalizes the label (case, internal whitespace), so take the marker as
    // written; otherwise the body after a label like `[^a  b]:` would be lost.
    let line0 = line0.trim_start();
    let marker = raw_footnote_marker(line0).map_or_else(|| format!("[^{label}]:"), str::to_string);
    // GFM rejects whitespace and escaped brackets in footnote labels. Wrapping these
    // definitions can create ordinary link definitions in that renderer.
    if marker[2..marker.len() - 2].contains(|c: char| c.is_whitespace() || c == '\\') {
        return raw_lines_verbatim(raw);
    }
    let first_prefix = format!("{marker} ");

    let rem0 = line0
        .strip_prefix(marker.as_str())
        .map(|r| r.trim_start_matches([' ', '\t']))
        .unwrap_or("");

    // A lazy continuation line (indented less than the footnote body) continues the previous
    // paragraph. Re-indenting it as part of the body could change its meaning (`===` would
    // become a setext underline), so join it onto that paragraph instead, as for list items.
    let breaks = soft_breaks(raw);
    let hard_breaks: Vec<_> = Parser::new_ext(raw, parser_options())
        .into_offset_iter()
        .filter_map(|(event, range)| matches!(event, Event::HardBreak).then_some(range))
        .collect();
    let mut offset = raw_lines[0].len() + 1;
    let mut lazy_lines = Vec::with_capacity(raw_lines.len());
    for &cont in &raw_lines[1..] {
        if preprocess::leading_ws_columns(cont, TAB_STOP).0 < 4
            && follows_soft_break(&hard_breaks, offset)
        {
            // Joining would discard the break; indenting can create a heading or table.
            return raw_lines_verbatim(raw);
        }
        lazy_lines.push(
            preprocess::leading_ws_columns(cont, TAB_STOP).0 < 4
                && follows_soft_break(&breaks, offset),
        );
        offset += cont.len() + 1;
    }
    if options.wrap == crate::options::WrapMode::Preserve && lazy_lines.contains(&true) {
        // Keep the layout, as for lazy list items in preserve mode.
        return raw_lines_verbatim(raw);
    }

    let mut body_lines = vec![rem0.to_string()];
    for (&cont, lazy) in raw_lines[1..].iter().zip(lazy_lines) {
        if lazy {
            let previous = body_lines.last_mut().expect("body has a first line");
            previous.push(' ');
            previous.push_str(cont.trim_start_matches([' ', '\t']));
        } else if cont.trim().is_empty() {
            body_lines.push(String::new());
        } else {
            body_lines.push(dedent_columns(cont, 4, TAB_STOP));
        }
    }

    let body = body_lines.join("\n");
    if body.trim().is_empty() {
        return vec![marker];
    }

    let indent = ContainerIndent {
        para: 4,
        block: 4,
        content: 4,
    };
    format_prefixed_container(
        &body,
        &first_prefix,
        indent,
        width,
        false,
        is_markdeep,
        options,
    )
}

fn format_prefixed_container(
    body: &str,
    first_prefix: &str,
    indent: ContainerIndent,
    width: usize,
    is_root: bool,
    is_markdeep: bool,
    options: &FormattingOptions,
) -> Vec<String> {
    let first_line_blank = body.split('\n').next().is_some_and(|l| l.trim().is_empty());
    let block_inner_width = width.saturating_sub(indent.block).max(1);

    let (units, body_starts_on_marker_line) = if first_line_blank {
        let trimmed_body = body.trim_start_matches('\n');
        if trimmed_body.trim().is_empty() {
            return vec![first_prefix.trim_end().to_string()];
        }
        let units = format_block_units(
            trimmed_body,
            1,
            block_inner_width,
            false,
            is_markdeep,
            options,
            None,
        );
        (units, false)
    } else {
        let ov = FirstParaOverride {
            first_prefix,
            first_width: width.saturating_sub(first_prefix.len()).max(1),
            rest_indent: indent.para,
            rest_width: width.saturating_sub(indent.para).max(1),
            root_continuations: is_root && indent.para == 0,
        };
        let units = format_block_units(
            body,
            1,
            block_inner_width,
            false,
            is_markdeep,
            options,
            Some(ov),
        );
        (units, true)
    };

    if units.is_empty() {
        return vec![first_prefix.trim_end().to_string()];
    }

    let mut out = Vec::new();
    if !body_starts_on_marker_line {
        out.push(first_prefix.trim_end().to_string());
    }
    let block_pad = " ".repeat(indent.block);
    let code_pad = " ".repeat(indent.content);
    // A leading paragraph was already formatted with the marker prefix (via `FirstParaOverride`).
    let first_unit_handled_prefix = body_starts_on_marker_line && units[0].is_paragraph;

    for (u_idx, unit) in units.iter().enumerate() {
        if unit.lines.is_empty() {
            continue;
        }
        if u_idx > 0 {
            let prev = &units[u_idx - 1];
            let no_blank =
                prev.tight_after || unit.tight_before || unit.start_line == prev.end_line + 1;
            if !no_blank && !out.is_empty() {
                out.push(String::new());
            }
        }

        if u_idx == 0 && first_unit_handled_prefix {
            out.extend(unit.lines.iter().cloned());
            continue;
        }
        let pad = if unit.keeps_source_indent {
            &code_pad
        } else {
            &block_pad
        };
        for (l_idx, l) in unit.lines.iter().enumerate() {
            if body_starts_on_marker_line && u_idx == 0 && l_idx == 0 {
                if l.is_empty() {
                    out.push(first_prefix.trim_end().to_string());
                } else {
                    out.push(format!("{first_prefix}{l}"));
                }
            } else if l.is_empty() {
                out.push(String::new());
            } else {
                out.push(format!("{pad}{l}"));
            }
        }
    }

    out
}
