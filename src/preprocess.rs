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

use crate::markdown::{parser_options, TAB_STOP};
use pulldown_cmark::{Event, Parser, Tag};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreSegment {
    Frontmatter {
        range: Range<usize>,
        start_line: usize,
        end_line: usize,
    },
    Verbatim {
        range: Range<usize>,
        start_line: usize,
        end_line: usize,
    },
    StandaloneLine {
        text: String,
        blank_before: bool,
        blank_after: bool,
        range: Range<usize>,
        start_line: usize,
        end_line: usize,
    },
    Admonition {
        indent: usize,
        header: String,
        body: String,
        range: Range<usize>,
        start_line: usize,
        end_line: usize,
    },
    DefinitionList {
        term: String,
        blank_after_term: bool,
        definitions: Vec<(String, String)>,
        range: Range<usize>,
        start_line: usize,
        end_line: usize,
    },
    MarkdownChunk {
        range: Range<usize>,
        start_line: usize,
    },
}

#[derive(Debug, Clone, Copy)]
struct LineSpan<'a> {
    line_no: usize,
    start: usize,
    end_excl: usize,
    raw: &'a str,
    is_blank: bool,
}

#[derive(Debug, Clone, Copy)]
struct FenceState {
    ch: u8,
    count: usize,
}

const EXCLUDED_HTML_TAGS: &[&str] = &[
    "script",
    "style",
    "pre",
    "textarea",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "colgroup",
    "col",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "svg",
    "math",
    "iframe",
    "canvas",
    "head",
    "html",
    "body",
    "form",
    "select",
    "option",
    "hr",
    "br",
    "img",
    "input",
    "meta",
    "link",
    "blockquote",
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "aside",
    "main",
    "figure",
    "figcaption",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
];

pub fn has_markdeep_markers(text: &str) -> bool {
    let mut fence: Option<FenceState> = None;
    for raw_line in text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if let Some(f) = fence {
            if is_fence_close(line, f.ch, f.count) {
                fence = None;
            }
            continue;
        }
        let lead = line.bytes().take_while(|&b| b == b' ').count();
        if lead > 3 || line.starts_with('\t') {
            continue;
        }
        let trimmed_start = &line[lead..];
        if let Some(opened) = parse_fence_open(trimmed_start) {
            fence = Some(opened);
            continue;
        }
        let trimmed = trimmed_start.trim_end();
        if !trimmed.starts_with('<') {
            continue;
        }
        if trimmed.starts_with("<!-- Markdeep:")
            || ((trimmed.contains("<script") || trimmed.contains("<style"))
                && trimmed.contains("markdeep"))
        {
            return true;
        }
    }
    false
}

/// Counts leading horizontal indentation columns (`' '` and `\t` up to `tab_width`), stopping at
/// the first non-whitespace character. Returns `(columns, byte_offset)`.
pub fn leading_ws_columns(line: &str, tab_width: usize) -> (usize, usize) {
    let tw = tab_width.max(1);
    let mut cols = 0usize;
    let mut bytes = 0usize;
    for b in line.bytes() {
        match b {
            b' ' => {
                cols += 1;
                bytes += 1;
            }
            b'\t' => {
                let advance = tw - (cols % tw);
                cols += advance;
                bytes += 1;
            }
            _ => break,
        }
    }
    (cols, bytes)
}

/// Removes up to `target_cols` columns of leading whitespace from `line`, partially expanding a
/// tab if the column boundary falls inside it.
pub fn dedent_columns(line: &str, target_cols: usize, tab_width: usize) -> String {
    if target_cols == 0 || line.is_empty() {
        return line.to_string();
    }
    let tw = tab_width.max(1);
    let mut cols = 0usize;
    let mut idx = 0usize;
    let bytes = line.as_bytes();

    while idx < bytes.len() && cols < target_cols {
        match bytes[idx] {
            b' ' => {
                cols += 1;
                idx += 1;
            }
            b'\t' => {
                let advance = tw - (cols % tw);
                if cols + advance > target_cols {
                    let leftover = cols + advance - target_cols;
                    let mut out = " ".repeat(leftover);
                    out.push_str(&line[idx + 1..]);
                    return out;
                }
                cols += advance;
                idx += 1;
            }
            _ => break,
        }
    }

    line[idx..].to_string()
}

pub fn preprocess(text: &str, is_root: bool, is_markdeep: bool) -> Vec<PreSegment> {
    let tab_width = TAB_STOP;
    let (hard_protected, soft_protected) = protected_ranges(text);
    let raw_text = Intervals::new(raw_text_regions(text));
    let lines = split_line_spans(text);
    if lines.is_empty() {
        return Vec::new();
    }

    let mut segments: Vec<PreSegment> = Vec::new();
    let mut chunk_start_idx: Option<usize> = None;
    let mut chunk_last_non_blank: Option<usize> = None;
    let mut fence: Option<FenceState> = None;
    let mut idx = 0usize;

    let flush_chunk = |segments: &mut Vec<PreSegment>,
                       chunk_start: &mut Option<usize>,
                       chunk_last: &mut Option<usize>| {
        if let (Some(start_i), Some(end_i)) = (*chunk_start, *chunk_last) {
            if start_i <= end_i {
                segments.push(PreSegment::MarkdownChunk {
                    range: lines[start_i].start..lines[end_i].end_excl,
                    start_line: lines[start_i].line_no,
                });
            }
        }
        *chunk_start = None;
        *chunk_last = None;
    };

    let verbatim = |start_i: usize, end_i: usize| PreSegment::Verbatim {
        range: lines[start_i].start..lines[end_i].end_excl,
        start_line: lines[start_i].line_no,
        end_line: lines[end_i].line_no,
    };

    while idx < lines.len() {
        let line = lines[idx];

        // 1. Root-level line 1 frontmatter and Markdeep title header.
        if is_root && idx == 0 {
            if let Some(end_idx) = match_frontmatter(&lines) {
                segments.push(PreSegment::Frontmatter {
                    range: lines[0].start..lines[end_idx].end_excl,
                    start_line: lines[0].line_no,
                    end_line: lines[end_idx].line_no,
                });
                idx = end_idx + 1;
                continue;
            }
            if is_markdeep {
                if let Some(end_idx) = match_markdeep_title_block(&lines) {
                    segments.push(verbatim(0, end_idx));
                    idx = end_idx + 1;
                    continue;
                }
            }
        }

        if let Some(region) = raw_text.containing(line.start) {
            flush_chunk(
                &mut segments,
                &mut chunk_start_idx,
                &mut chunk_last_non_blank,
            );
            let end_idx = (idx..lines.len())
                .take_while(|&i| lines[i].start < region.end)
                .last()
                .unwrap_or(idx);
            segments.push(verbatim(idx, end_idx));
            idx = end_idx + 1;
            continue;
        }

        if is_markdeep {
            if let Some(end_idx) = match_markdeep_footer(&lines, idx) {
                flush_chunk(
                    &mut segments,
                    &mut chunk_start_idx,
                    &mut chunk_last_non_blank,
                );
                segments.push(verbatim(idx, end_idx));
                idx = end_idx + 1;
                continue;
            }
        }

        // Containers are preprocessed after their markers have been removed by the formatter.
        // Opaque HTML and code must reach the parser intact, including their trailing blank lines.
        let (_, lead_bytes) = leading_ws_columns(line.raw, TAB_STOP);
        let content_start = line.start + lead_bytes;
        let probe = if line.is_blank {
            line.start
        } else {
            content_start
        };
        // A prompt tag at column 0 inside a list, quote, footnote, or paragraph (inline HTML) is
        // a lazy continuation line; keeping it on its own line leaves the structure unchanged.
        let prompt_tag_line =
            lead_bytes == 0 && !line.is_blank && is_standalone_prompt_xml_tag(line.raw.trim());
        if hard_protected.overlaps(content_start, probe)
            || (!prompt_tag_line && soft_protected.overlaps(content_start, probe))
        {
            chunk_start_idx.get_or_insert(idx);
            chunk_last_non_blank = Some(idx);
            idx += 1;
            continue;
        }

        // 2. Inside active code fence: track close fence, never match directives.
        if let Some(f) = fence {
            if chunk_start_idx.is_none() {
                chunk_start_idx = Some(idx);
            }
            chunk_last_non_blank = Some(idx);
            if is_fence_close(line.raw, f.ch, f.count) {
                fence = None;
            }
            idx += 1;
            continue;
        }

        let (lead_cols, lead_bytes) = leading_ws_columns(line.raw, tab_width);
        let top_level_candidate = lead_cols <= 3 && !line.raw.starts_with('\t');

        if top_level_candidate && !line.is_blank {
            let trimmed_start = &line.raw[lead_bytes..];
            let trimmed = line.raw.trim();

            // Check if this line opens a fenced code block (which protects everything inside).
            if let Some(opened) = parse_fence_open(trimmed_start) {
                if chunk_start_idx.is_none() {
                    chunk_start_idx = Some(idx);
                }
                chunk_last_non_blank = Some(idx);
                fence = Some(opened);
                idx += 1;
                continue;
            }

            // `<!-- markfmt:off -->` ... `<!-- markfmt:on -->`
            if is_formatter_off_comment(trimmed) {
                flush_chunk(
                    &mut segments,
                    &mut chunk_start_idx,
                    &mut chunk_last_non_blank,
                );
                let end_idx = find_formatter_on_line(&lines, idx + 1);
                segments.push(verbatim(idx, end_idx));
                idx = end_idx + 1;
                continue;
            }

            // `<!-- prettier-ignore -->` / `<!-- markfmt-ignore-next -->`
            if is_ignore_next_comment(trimmed) {
                flush_chunk(
                    &mut segments,
                    &mut chunk_start_idx,
                    &mut chunk_last_non_blank,
                );
                let end_idx = find_ignore_next_end(&lines, idx);
                segments.push(verbatim(idx, end_idx));
                idx = end_idx + 1;
                continue;
            }

            // Display math `$$...$$` (only at the start of a block: inside a paragraph, `$$...$$`
            // is inline math and part of the paragraph text).
            let starts_block =
                idx == 0 || lines[idx - 1].is_blank || chunk_last_non_blank.is_none();
            if let Some(end_idx) = match_display_math(&lines, idx).filter(|_| starts_block) {
                flush_chunk(
                    &mut segments,
                    &mut chunk_start_idx,
                    &mut chunk_last_non_blank,
                );
                segments.push(verbatim(idx, end_idx));
                idx = end_idx + 1;
                continue;
            }

            // Markdeep-only constructs
            if is_markdeep {
                if let Some(end_idx) = match_markdeep_diagram(&lines, idx) {
                    flush_chunk(
                        &mut segments,
                        &mut chunk_start_idx,
                        &mut chunk_last_non_blank,
                    );
                    segments.push(verbatim(idx, end_idx));
                    idx = end_idx + 1;
                    continue;
                }

                if let Some((admon_seg, end_idx)) =
                    match_markdeep_admonition(&lines, idx, lead_cols, trimmed_start, tab_width)
                {
                    flush_chunk(
                        &mut segments,
                        &mut chunk_start_idx,
                        &mut chunk_last_non_blank,
                    );
                    segments.push(admon_seg);
                    idx = end_idx + 1;
                    continue;
                }

                let prev_separated =
                    idx == 0 || lines[idx - 1].is_blank || chunk_last_non_blank.is_none();
                if prev_separated {
                    if let Some((def_seg, end_idx)) = match_definition_list(&lines, idx, tab_width)
                    {
                        flush_chunk(
                            &mut segments,
                            &mut chunk_start_idx,
                            &mut chunk_last_non_blank,
                        );
                        segments.push(def_seg);
                        idx = end_idx + 1;
                        continue;
                    }
                }
            }

            // Standalone `@path` agent import, or standalone XML / prompt tag line (prevents
            // CommonMark HTML Block Rule 7 from swallowing the prose that follows).
            if is_standalone_at_import(trimmed) || is_standalone_prompt_xml_tag(trimmed) {
                flush_chunk(
                    &mut segments,
                    &mut chunk_start_idx,
                    &mut chunk_last_non_blank,
                );
                let blank_before = idx > 0 && lines[idx - 1].is_blank;
                let blank_after = idx + 1 < lines.len() && lines[idx + 1].is_blank;
                segments.push(PreSegment::StandaloneLine {
                    text: trimmed.to_string(),
                    blank_before,
                    blank_after,
                    range: line.start..line.end_excl,
                    start_line: line.line_no,
                    end_line: line.line_no,
                });
                idx += 1;
                continue;
            }
        }

        // Internal blank lines stay inside the chunk; flush trims trailing ones.
        if !line.is_blank {
            if chunk_start_idx.is_none() {
                chunk_start_idx = Some(idx);
            }
            chunk_last_non_blank = Some(idx);
        }
        idx += 1;
    }

    flush_chunk(
        &mut segments,
        &mut chunk_start_idx,
        &mut chunk_last_non_blank,
    );
    segments
}

fn split_line_spans(text: &str) -> Vec<LineSpan<'_>> {
    if text.is_empty() {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut line_no = 1usize;

    while pos < len {
        let start = pos;
        while pos < len && bytes[pos] != b'\n' {
            pos += 1;
        }
        let end_excl = pos;
        let raw = &text[start..end_excl];
        let is_blank = raw.bytes().all(|b| b == b' ' || b == b'\t');
        out.push(LineSpan {
            line_no,
            start,
            end_excl,
            raw,
            is_blank,
        });
        if pos < len && bytes[pos] == b'\n' {
            pos += 1;
        }
        line_no += 1;
    }

    out
}

fn match_frontmatter(lines: &[LineSpan<'_>]) -> Option<usize> {
    if lines.len() < 2 {
        return None;
    }
    let first = lines[0].raw;
    if first.starts_with([' ', '\t']) {
        return None;
    }
    let first_trimmed = first.trim_end_matches([' ', '\t']);

    if first_trimmed == "---" {
        let second = lines[1];
        if second.is_blank {
            return None;
        }
        let second_trimmed = second.raw.trim();
        let plausible_yaml = second_trimmed == "---"
            || second_trimmed == "..."
            || second_trimmed.starts_with('#')
            || second.raw.contains(':');
        if !plausible_yaml {
            return None;
        }
        for (idx, line) in lines.iter().enumerate().skip(1) {
            if !line.raw.starts_with([' ', '\t']) {
                let t = line.raw.trim_end_matches([' ', '\t']);
                if t == "---" || t == "..." {
                    return Some(idx);
                }
            }
        }
        return None;
    }

    if first_trimmed == "+++" {
        let second = lines[1];
        if second.is_blank {
            return None;
        }
        let second_trimmed = second.raw.trim();
        let plausible_toml = second_trimmed == "+++"
            || second_trimmed.starts_with('#')
            || second.raw.contains('=')
            || second_trimmed.starts_with('[');
        if !plausible_toml {
            return None;
        }
        for (idx, line) in lines.iter().enumerate().skip(1) {
            if !line.raw.starts_with([' ', '\t']) {
                let t = line.raw.trim_end_matches([' ', '\t']);
                if t == "+++" {
                    return Some(idx);
                }
            }
        }
        return None;
    }

    None
}

fn match_markdeep_title_block(lines: &[LineSpan<'_>]) -> Option<usize> {
    let first = lines[0].raw;
    let lead = first.bytes().take_while(|&b| b == b' ').count();
    if lead > 3 || first.starts_with('\t') {
        return None;
    }
    let trimmed = first.trim();
    if !trimmed.starts_with("**") || trimmed.len() <= 4 {
        return None;
    }
    let inner = &trimmed[2..];
    if !inner.contains("**") {
        return None;
    }
    // Avoid mistaking an ordinary opening sentence `**Note:** This starts...` for a title.
    let next_indented = lines
        .get(1)
        .is_some_and(|l| !l.is_blank && (l.raw.starts_with(' ') || l.raw.starts_with('\t')));
    if !trimmed.ends_with("**") && !next_indented {
        return None;
    }

    let mut end_idx = 0usize;
    for (idx, line) in lines.iter().enumerate().skip(1) {
        if line.is_blank {
            break;
        }
        end_idx = idx;
    }
    Some(end_idx)
}

fn parse_fence_open(trimmed_start: &str) -> Option<FenceState> {
    let bytes = trimmed_start.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let ch = bytes[0];
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let count = bytes.iter().take_while(|&&b| b == ch).count();
    if count < 3 {
        return None;
    }
    let info = &trimmed_start[count..];
    if ch == b'`' && info.contains('`') {
        return None;
    }
    Some(FenceState { ch, count })
}

fn is_fence_close(raw: &str, ch: u8, open_count: usize) -> bool {
    let lead = raw.bytes().take_while(|&b| b == b' ').count();
    if lead > 3 || raw.starts_with('\t') {
        return false;
    }
    let rest = &raw[lead..];
    let count = rest.bytes().take_while(|&b| b == ch).count();
    if count < open_count {
        return false;
    }
    rest[count..].bytes().all(|b| b == b' ' || b == b'\t')
}

fn is_formatter_off_comment(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    lower == "<!-- markfmt:off -->" || lower == "<!--markfmt:off-->"
}

fn is_formatter_on_comment(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    lower == "<!-- markfmt:on -->" || lower == "<!--markfmt:on-->"
}

fn find_formatter_on_line(lines: &[LineSpan<'_>], start: usize) -> usize {
    for (idx, line) in lines.iter().enumerate().skip(start) {
        if is_formatter_on_comment(line.raw.trim()) {
            return idx;
        }
    }
    lines.len() - 1
}

fn is_ignore_next_comment(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    lower == "<!-- prettier-ignore -->" || lower == "<!-- markfmt-ignore-next -->"
}

fn find_ignore_next_end(lines: &[LineSpan<'_>], comment_idx: usize) -> usize {
    let Some(start) = (comment_idx + 1..lines.len()).find(|&i| !lines[i].is_blank) else {
        return comment_idx;
    };
    let source = lines[start..]
        .iter()
        .map(|line| line.raw)
        .collect::<Vec<_>>()
        .join("\n");
    let mut depth = 0usize;
    for (event, range) in Parser::new_ext(&source, parser_options()).into_offset_iter() {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            // List ranges may extend into indentation on the following line.
            let end = source[..range.end]
                .trim_end_matches([' ', '\t', '\n'])
                .len();
            return start + source[..end].bytes().filter(|&b| b == b'\n').count();
        }
    }
    lines.len() - 1
}

/// Raw-text HTML can start inline and continue through later Markdown blocks. Preserve the
/// enclosing blocks too: their generated HTML is inside the raw-text element, so whitespace
/// there is significant even though the Markdown parser still emits ordinary text events.
fn raw_text_regions(text: &str) -> Vec<Range<usize>> {
    let mut depth = 0usize;
    let mut root = 0..0;
    let mut open: Option<(String, usize)> = None;
    let mut regions = Vec::new();
    for (event, range) in Parser::new_ext(text, parser_options()).into_offset_iter() {
        match &event {
            Event::Start(_) => {
                if depth == 0 {
                    root = range.clone();
                }
                depth += 1;
            }
            Event::End(_) => depth -= 1,
            Event::InlineHtml(html) | Event::Html(html) => {
                for (name, closing) in html_tags(html) {
                    if closing && open.as_ref().is_some_and(|(tag, _)| *tag == name) {
                        let (_, start) = open.take().unwrap();
                        // List ranges may extend over the blank lines that follow them; keeping
                        // those verbatim would add a blank line on every pass, so end the region
                        // after the last non-blank line (including its line ending).
                        let content_end = text[..root.end]
                            .trim_end_matches([' ', '\t', '\r', '\n'])
                            .len();
                        let end = text[content_end..root.end]
                            .find('\n')
                            .map_or(root.end, |i| content_end + i + 1);
                        regions.push(start..end.max(start));
                    } else if !closing
                        && open.is_none()
                        && matches!(
                            name.as_str(),
                            "pre" | "code" | "script" | "style" | "textarea"
                        )
                    {
                        let start = text[..root.start].rfind('\n').map_or(0, |i| i + 1);
                        open = Some((name, start));
                    }
                }
            }
            _ => {}
        }
    }
    if let Some((_, start)) = open {
        regions.push(start..text.len());
    }
    let mut merged: Vec<Range<usize>> = Vec::new();
    for region in regions {
        if let Some(last) = merged.last_mut().filter(|last| last.end >= region.start) {
            last.end = last.end.max(region.end);
        } else {
            merged.push(region);
        }
    }
    merged
}

/// Read tags without mistaking quoted attribute values or comments for raw-text elements.
fn html_tags(mut html: &str) -> Vec<(String, bool)> {
    let mut tags = Vec::new();
    while let Some(start) = html.find('<') {
        html = &html[start..];
        if html.starts_with("<!--") {
            let Some(end) = html.find("-->") else { break };
            html = &html[end + 3..];
            continue;
        }
        let mut quote = None;
        let Some(end) = html.char_indices().find_map(|(i, ch)| {
            if let Some(q) = quote {
                if ch == q {
                    quote = None;
                }
            } else if matches!(ch, '\'' | '"') {
                quote = Some(ch);
            } else if ch == '>' {
                return Some(i + 1);
            }
            None
        }) else {
            break;
        };
        let tag = &html[..end];
        let name = tag
            .trim_start_matches(['<', '/'])
            .split(|c: char| !c.is_ascii_alphanumeric())
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        tags.push((name, tag.starts_with("</")));
        html = &html[end..];
    }
    tags
}

/// Ranges where preprocessing must not recognize extensions, as `(hard, soft)`. Hard ranges are
/// literals and opaque HTML. Soft ranges are containers and inline HTML, where a standalone
/// prompt tag at column 0 is still recognized. Prompt-tag HTML blocks are deliberately left
/// available: their contents are Markdown in agent files.
fn protected_ranges(text: &str) -> (Intervals, Intervals) {
    let mut hard = Vec::new();
    let mut soft = Vec::new();
    for (event, range) in Parser::new_ext(text, parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_))
            | Event::Code(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => hard.push(range),
            Event::Start(Tag::HtmlBlock) => {
                let first = text[range.clone()].lines().next().unwrap_or("").trim();
                if !is_standalone_prompt_xml_tag(first)
                    && !is_formatter_off_comment(first)
                    && !is_ignore_next_comment(first)
                {
                    hard.push(range);
                }
            }
            Event::Start(Tag::List(_) | Tag::BlockQuote(_) | Tag::FootnoteDefinition(_))
            | Event::InlineHtml(_) => soft.push(range),
            _ => {}
        }
    }
    (Intervals::new(hard), Intervals::new(soft))
}

/// A union of ranges, merged and sorted for `O(log n)` queries.
struct Intervals(Vec<Range<usize>>);

impl Intervals {
    fn new(mut ranges: Vec<Range<usize>>) -> Self {
        ranges.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        Self(merged)
    }

    /// Whether some range starts at or before `start` and extends past `probe` (`probe <= start`).
    fn overlaps(&self, start: usize, probe: usize) -> bool {
        let first_ending_after = self.0.partition_point(|range| range.end <= probe);
        self.0
            .get(first_ending_after)
            .is_some_and(|range| range.start <= start)
    }

    /// The merged range containing `offset`, if any.
    fn containing(&self, offset: usize) -> Option<&Range<usize>> {
        let index = self.0.partition_point(|range| range.end <= offset);
        self.0.get(index).filter(|range| range.start <= offset)
    }
}

fn match_display_math(lines: &[LineSpan<'_>], start: usize) -> Option<usize> {
    let trimmed = lines[start].raw.trim();
    if !trimmed.starts_with("$$") {
        return None;
    }
    if trimmed.len() >= 4 && trimmed.ends_with("$$") {
        return Some(start);
    }
    if trimmed == "$$" {
        for (idx, line) in lines.iter().enumerate().skip(start + 1) {
            if line.raw.trim().ends_with("$$") {
                return Some(idx);
            }
        }
    }
    None
}

fn match_markdeep_footer(lines: &[LineSpan<'_>], start: usize) -> Option<usize> {
    let trimmed_start_line = lines[start].raw.trim();
    if !trimmed_start_line.starts_with('<') {
        return None;
    }

    let starts_footer = trimmed_start_line.starts_with("<!-- Markdeep:")
        || (trimmed_start_line.contains("<script") && trimmed_start_line.contains("markdeep"))
        || (trimmed_start_line.starts_with("<style class=\"fallback\"")
            && (start + 1..lines.len().min(start + 5)).any(|j| {
                !lines[j].is_blank && {
                    let tj = lines[j].raw.trim();
                    tj.starts_with('<') && (tj.contains("markdeep") || tj.contains("Markdeep"))
                }
            }));
    if !starts_footer {
        return None;
    }

    let mut end = start;
    let mut i = start;
    while i < lines.len() {
        if lines[i].is_blank {
            break;
        }
        let t = lines[i].raw.trim();
        if !t.starts_with('<') && t != "-->" {
            break;
        }

        if t.contains("markdeep")
            || t.contains("Markdeep")
            || t.contains("<!-- Markdeep:")
            || t.contains("markdeepOptions")
            || t == "-->"
        {
            end = i;
            if (t.contains("<script") && !t.contains("</script>"))
                || (t.contains("<!--") && !t.contains("-->"))
            {
                while i + 1 < lines.len() && !lines[i + 1].is_blank {
                    i += 1;
                    end = i;
                    let cur = lines[i].raw;
                    if cur.contains("</script>") || cur.contains("-->") {
                        break;
                    }
                }
            }
            i += 1;
            continue;
        }

        if (i + 1..lines.len().min(i + 4)).any(|j| {
            !lines[j].is_blank && {
                let tj = lines[j].raw.trim();
                tj.starts_with('<') && (tj.contains("markdeep") || tj.contains("Markdeep"))
            }
        }) {
            end = i;
            i += 1;
            continue;
        }

        break;
    }

    Some(end)
}

pub(crate) fn is_markdeep_diagram_border(trimmed: &str) -> bool {
    trimmed.len() >= 4
        && trimmed.bytes().all(|b| b == b'*' || b == b'+')
        && trimmed.bytes().filter(|&b| b == b'*').count() >= 3
}

fn match_markdeep_diagram(lines: &[LineSpan<'_>], start: usize) -> Option<usize> {
    if !is_markdeep_diagram_border(lines[start].raw.trim()) {
        return None;
    }
    if start + 1 >= lines.len() || lines[start + 1].is_blank {
        return None;
    }

    let mut close_idx = None;
    for (idx, line) in lines.iter().enumerate().skip(start + 1) {
        if line.is_blank {
            break;
        }
        if is_markdeep_diagram_border(line.raw.trim()) {
            close_idx = Some(idx);
            break;
        }
    }
    let mut end_idx = close_idx?;

    // Include optional `[Figure ...]` caption line immediately after the closing border.
    if end_idx + 1 < lines.len() {
        let next_t = lines[end_idx + 1].raw.trim();
        if next_t.starts_with('[') && next_t.ends_with(']') && !next_t.contains("]:") {
            end_idx += 1;
        }
    }
    Some(end_idx)
}

fn match_markdeep_admonition(
    lines: &[LineSpan<'_>],
    start: usize,
    indent: usize,
    trimmed_start: &str,
    tab_width: usize,
) -> Option<(PreSegment, usize)> {
    let after = trimmed_start.strip_prefix("!!!")?;
    if !(after.is_empty()
        || after.starts_with(' ')
        || after.starts_with('\t')
        || after.starts_with(':'))
    {
        return None;
    }

    let header = trimmed_start.trim_end().to_string();
    let required_indent = indent + 4;
    let mut end_idx = start;
    let mut scan = start + 1;

    while scan < lines.len() {
        if lines[scan].is_blank {
            scan += 1;
            continue;
        }
        let (cols, _) = leading_ws_columns(lines[scan].raw, tab_width);
        if cols >= required_indent {
            end_idx = scan;
            scan += 1;
        } else {
            break;
        }
    }

    let mut body_lines = Vec::new();
    for line in &lines[start + 1..=end_idx] {
        if line.is_blank {
            body_lines.push(String::new());
        } else {
            body_lines.push(dedent_columns(line.raw, required_indent, tab_width));
        }
    }
    let body = body_lines.join("\n");

    Some((
        PreSegment::Admonition {
            indent,
            header,
            body,
            range: lines[start].start..lines[end_idx].end_excl,
            start_line: lines[start].line_no,
            end_line: lines[end_idx].line_no,
        },
        end_idx,
    ))
}

fn is_candidate_term_line(line: &str) -> bool {
    let t = line.trim_start();
    if t.is_empty()
        || t.starts_with('#')
        || t.starts_with('>')
        || t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("+ ")
        || t.starts_with(':')
        || t.starts_with("```")
        || t.starts_with("~~~")
        || t.starts_with('|')
        || t.starts_with('<')
        || t.starts_with("!!!")
        || t.starts_with("$$")
        || is_markdeep_diagram_border(line.trim())
    {
        return false;
    }
    if t.starts_with('[') && t.contains("]:") {
        return false;
    }
    // Exclude ordered list items `1. ` or `1) `
    let digit_count = t.bytes().take_while(|b| b.is_ascii_digit()).count();
    if (1..=9).contains(&digit_count) {
        let rest = &t.as_bytes()[digit_count..];
        if rest.len() >= 2 && (rest[0] == b'.' || rest[0] == b')') && rest[1] == b' ' {
            return false;
        }
    }
    true
}

/// Parses `:   body` definition marker. Returns `(marker_cols, canonical_marker_prefix, remainder)`.
fn parse_definition_marker(line: &str) -> Option<(usize, String, String)> {
    let lead = line.bytes().take_while(|&b| b == b' ').count();
    if lead > 3 || line.starts_with('\t') {
        return None;
    }
    let rest = &line[lead..];
    if !rest.starts_with(':') {
        return None;
    }
    let after_colon = &rest[1..];
    let spaces = after_colon.bytes().take_while(|&b| b == b' ').count();
    if spaces == 0 && !after_colon.starts_with('\t') {
        return None;
    }
    let marker_spaces = if spaces > 0 { spaces.min(4) } else { 3 };
    let marker_cols = lead + 1 + marker_spaces;
    let marker_prefix = format!(":{}", " ".repeat(marker_spaces));
    let remainder = if spaces > 0 {
        rest[1 + marker_spaces..].to_string()
    } else {
        rest[2..].to_string()
    };
    Some((marker_cols, marker_prefix, remainder))
}

fn match_definition_list(
    lines: &[LineSpan<'_>],
    term_idx: usize,
    tab_width: usize,
) -> Option<(PreSegment, usize)> {
    if !is_candidate_term_line(lines[term_idx].raw) {
        return None;
    }

    let (blank_after_term, mut def_idx) = if term_idx + 1 < lines.len()
        && parse_definition_marker(lines[term_idx + 1].raw).is_some()
    {
        (false, term_idx + 1)
    } else if term_idx + 2 < lines.len()
        && lines[term_idx + 1].is_blank
        && parse_definition_marker(lines[term_idx + 2].raw).is_some()
    {
        (true, term_idx + 2)
    } else {
        return None;
    };

    let term = lines[term_idx].raw.trim().to_string();
    let mut definitions: Vec<(String, String)> = Vec::new();
    let mut end_idx = term_idx;

    while def_idx < lines.len() {
        let Some((marker_cols, marker_prefix, first_rem)) =
            parse_definition_marker(lines[def_idx].raw)
        else {
            break;
        };

        let mut body_lines = vec![first_rem];
        let mut def_last = def_idx;
        let mut scan = def_idx + 1;
        let mut saw_blank = false;

        while scan < lines.len() {
            if lines[scan].is_blank {
                saw_blank = true;
                scan += 1;
                continue;
            }
            if parse_definition_marker(lines[scan].raw).is_some() {
                break;
            }
            let (cols, _) = leading_ws_columns(lines[scan].raw, tab_width);
            if saw_blank {
                if cols >= marker_cols {
                    // Normalize blank line(s) inside the same definition before an indented continuation block.
                    if body_lines.last().is_some_and(|l| !l.is_empty()) {
                        body_lines.push(String::new());
                    }
                    body_lines.push(dedent_columns(lines[scan].raw, marker_cols, tab_width));
                    def_last = scan;
                    saw_blank = false;
                    scan += 1;
                    continue;
                } else {
                    break;
                }
            } else {
                // Immediately following non-blank line without blank line: either indented or lazy continuation.
                body_lines.push(if cols >= marker_cols {
                    dedent_columns(lines[scan].raw, marker_cols, tab_width)
                } else {
                    lines[scan].raw.trim_start().to_string()
                });
                def_last = scan;
                scan += 1;
            }
        }

        end_idx = def_last;
        definitions.push((marker_prefix, body_lines.join("\n")));

        // Check whether another `:   ` definition follows (immediately or after blank line(s)).
        if scan < lines.len() && parse_definition_marker(lines[scan].raw).is_some() {
            def_idx = scan;
        } else {
            break;
        }
    }

    if definitions.is_empty() {
        return None;
    }

    Some((
        PreSegment::DefinitionList {
            term,
            blank_after_term,
            definitions,
            range: lines[term_idx].start..lines[end_idx].end_excl,
            start_line: lines[term_idx].line_no,
            end_line: lines[end_idx].line_no,
        },
        end_idx,
    ))
}

pub(crate) fn is_standalone_at_import(trimmed: &str) -> bool {
    trimmed.starts_with('@')
        && !trimmed.starts_with("@[")
        && trimmed.len() >= 3
        && !trimmed.chars().any(char::is_whitespace)
        && (trimmed.contains('/') || trimmed.contains('.'))
}

pub(crate) fn is_standalone_prompt_xml_tag(trimmed: &str) -> bool {
    if !trimmed.starts_with('<') || !trimmed.ends_with('>') {
        return false;
    }
    if trimmed.starts_with("<!--") || trimmed.starts_with("<!") || trimmed.starts_with("<?") {
        return false;
    }
    let Some(first_gt) = trimmed.find('>') else {
        return false;
    };

    if first_gt == trimmed.len() - 1 {
        // Single tag on the line: `<Tag>`, `<Tag attr="v">`, `<Tag/>`, or `</Tag>`. Autolinks
        // (`<https://e.x>`, `<urn:isbn:1>`, `<a@b.c>`) are inline; the reflow tokenizer uses the
        // same test to let them start a line.
        if crate::inline::starts_with_autolink(trimmed) {
            return false;
        }
        let inner = &trimmed[1..first_gt];
        let Some(tag_name) = extract_tag_name(inner) else {
            return false;
        };
        return !is_excluded_html_tag(tag_name);
    }

    // Single-line paired tag `<TagName>inline text</TagName>` or `<summary>...</summary>`
    let open_inner = &trimmed[1..first_gt];
    if open_inner.starts_with('/') || open_inner.ends_with('/') {
        return false;
    }
    let Some(open_name) = extract_tag_name(open_inner) else {
        return false;
    };
    if !open_name.eq_ignore_ascii_case("summary") && is_excluded_html_tag(open_name) {
        return false;
    }

    let expected_close = format!("</{open_name}>");
    if let Some(body) = trimmed[first_gt + 1..].strip_suffix(&expected_close) {
        return !body.contains('<');
    }
    false
}

fn extract_tag_name(inner: &str) -> Option<&str> {
    let trimmed = inner.trim();
    let without_slash = trimmed.strip_prefix('/').unwrap_or(trimmed).trim_start();
    let without_self_close = without_slash
        .strip_suffix('/')
        .unwrap_or(without_slash)
        .trim_end();
    if without_self_close.is_empty() {
        return None;
    }

    let name_end = without_self_close
        .find(|c: char| c.is_ascii_whitespace())
        .unwrap_or(without_self_close.len());
    let name = &without_self_close[..name_end];
    if !is_valid_xml_tag_name(name) {
        return None;
    }

    // Closing tags must not have stray tokens/attributes after tag name
    if trimmed.starts_with('/') && name_end != without_self_close.len() {
        return None;
    }

    Some(name)
}

fn is_valid_xml_tag_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':'))
}

fn is_excluded_html_tag(tag_name: &str) -> bool {
    let lower = tag_name.to_ascii_lowercase();
    EXCLUDED_HTML_TAGS.contains(&lower.as_str())
}
