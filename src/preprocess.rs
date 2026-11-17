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
        tight_before: bool,
        tight_after: bool,
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

impl PreSegment {
    /// The byte range of the source this segment covers.
    fn range(&self) -> Range<usize> {
        match self {
            PreSegment::Frontmatter { range, .. }
            | PreSegment::Verbatim { range, .. }
            | PreSegment::StandaloneLine { range, .. }
            | PreSegment::Admonition { range, .. }
            | PreSegment::DefinitionList { range, .. }
            | PreSegment::MarkdownChunk { range, .. } => range.clone(),
        }
    }
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

pub fn preprocess(
    text: &str,
    is_root: bool,
    is_markdeep: bool,
    commit: Option<CommitSyntax<'_>>,
) -> Vec<PreSegment> {
    let tab_width = TAB_STOP;
    let lines = split_line_spans(text);
    if lines.is_empty() {
        return Vec::new();
    }
    let frontmatter_end = if is_root {
        root_frontmatter_end(&lines, commit)
    } else {
        None
    };
    let body_offset = match frontmatter_end {
        Some(end) => lines.get(end + 1).map_or(text.len(), |line| line.start),
        None => 0,
    };
    let (hard_protected, soft_protected, top_blocks) =
        protected_ranges_from(&text[body_offset..], body_offset);
    let raw_text = Intervals::new(
        raw_text_regions(&text[body_offset..])
            .into_iter()
            .map(|r| (r.start + body_offset)..(r.end + body_offset))
            .collect(),
    );

    let mut segments: Vec<PreSegment> = Vec::new();
    let mut chunk_start_idx: Option<usize> = None;
    let mut chunk_last_non_blank: Option<usize> = None;
    let mut fence: Option<FenceState> = None;
    let mut idx = 0usize;
    let mut block_start = 0usize;
    // The opening commit subject is the first non-blank, non-comment line (matching `git
    // stripspace -s`), even after leading blank lines or template comments.
    let commit_subject_idx = commit
        .filter(|_| is_root && frontmatter_end.is_none())
        .and_then(|commit| {
            let first = lines.iter().position(|l| {
                !l.is_blank && !is_commit_comment_line(l.raw, commit.comment_prefix)
            })?;
            let spans_past_subject = raw_text
                .containing(lines[first].start)
                .is_some_and(|r| r.end > lines[first].end_excl + 1);
            (!spans_past_subject
                && is_single_line_commit_subject_at(&lines, first, commit.comment_prefix))
            .then_some(first)
        });
    // Git looks for trailers only above the first `---` patch divider line.
    let trailer_search_end = if commit.is_some() && is_root {
        lines
            .iter()
            .position(|line| is_git_patch_divider(line.raw))
            .unwrap_or(lines.len())
    } else {
        lines.len()
    };
    // The lines Git reads as the trailer block (`trailer.c`). Whatever they start with (a list
    // item, a quote), reflowing them could change Git's count of trailer and other lines, so
    // they are kept as written.
    let final_trailer_block = commit.filter(|_| is_root).and_then(|commit| {
        let raw: Vec<&str> = lines.iter().map(|l| l.raw).collect();
        let candidate = find_git_trailer_candidate(&raw, commit).filter(|c| c.is_block)?;
        let start = candidate.lines.start;
        let end = candidate.lines.rev().find(|&i| !lines[i].is_blank)?;
        Some((start, end))
    });
    // An old-style `Conflicts:` footer that Git skips when finding trailers: joining its
    // tab-indented paths onto `Conflicts:` would make it a trailer, so it stays as written.
    let conflicts_footer = commit.filter(|_| is_root).and_then(|commit| {
        let raw: Vec<&str> = lines.iter().map(|l| l.raw).collect();
        let log_end = git_log_message_end(&raw, commit);
        if !log_end.has_conflicts {
            return None;
        }
        let footer = log_end.end..log_end.cutoff;
        let start = footer.clone().find(|&i| !lines[i].is_blank)?;
        let end = footer.rev().find(|&i| !lines[i].is_blank)?;
        Some((start, end))
    });

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
        tight_before: false,
        tight_after: false,
    };
    // Commit-message lines kept verbatim inside the surrounding text, recording whether they
    // touch it without a blank line in between so rendering keeps them attached.
    let attached_verbatim = |start_i: usize, end_i: usize| PreSegment::Verbatim {
        range: lines[start_i].start..lines[end_i].end_excl,
        start_line: lines[start_i].line_no,
        end_line: lines[end_i].line_no,
        tight_before: start_i > 0 && !lines[start_i - 1].is_blank,
        tight_after: lines.get(end_i + 1).is_some_and(|next| !next.is_blank),
    };

    while idx < lines.len() {
        let line = lines[idx];

        // 1. Root-level frontmatter or Markdeep title header on line 1, or opening commit subject.
        if is_root && idx == 0 {
            if let Some(end_idx) = frontmatter_end {
                segments.push(PreSegment::Frontmatter {
                    range: lines[0].start..lines[end_idx].end_excl,
                    start_line: lines[0].line_no,
                    end_line: lines[end_idx].line_no,
                });
                idx = end_idx + 1;
                continue;
            }
            if is_markdeep && commit_subject_idx.is_none() {
                if let Some(end_idx) = match_markdeep_title_block(&lines) {
                    segments.push(verbatim(0, end_idx));
                    idx = end_idx + 1;
                    continue;
                }
            }
        }
        if commit_subject_idx == Some(idx) {
            flush_chunk(
                &mut segments,
                &mut chunk_start_idx,
                &mut chunk_last_non_blank,
            );
            block_start = idx;
            segments.push(attached_verbatim(idx, idx));
            idx += 1;
            continue;
        }

        if let Some(region) = raw_text.containing(line.start) {
            flush_chunk(
                &mut segments,
                &mut chunk_start_idx,
                &mut chunk_last_non_blank,
            );
            let Some(start_idx) = (idx..lines.len())
                .take_while(|&i| lines[i].start < region.end)
                .find(|&i| !lines[i].is_blank)
            else {
                idx += 1;
                continue;
            };
            let end_idx = (start_idx..lines.len())
                .take_while(|&i| lines[i].start < region.end)
                .last()
                .unwrap_or(start_idx);
            segments.push(verbatim(start_idx, end_idx));
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

        // Inside an active code fence: track close fence, never match directives or comments.
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

        if let Some((start, end)) = conflicts_footer.filter(|&(start, _)| start == idx) {
            if !hard_protected.overlaps(line.start, line.start) {
                // A footer attached to the block above is part of it for CommonMark (a lazy
                // paragraph continuation, say), so keep that whole block as written too:
                // formatting it alone would parse it in a different context.
                let first = if start > 0 && !lines[start - 1].is_blank {
                    rewind_to_block(
                        &mut segments,
                        &mut chunk_start_idx,
                        &mut chunk_last_non_blank,
                        &lines,
                        block_start,
                        idx,
                    )
                } else {
                    flush_chunk(
                        &mut segments,
                        &mut chunk_start_idx,
                        &mut chunk_last_non_blank,
                    );
                    start
                };
                segments.push(attached_verbatim(first, end));
                idx = end + 1;
                continue;
            }
        }
        if let Some((start, end)) = final_trailer_block.filter(|&(start, _)| start == idx) {
            if !hard_protected.overlaps(line.start, line.start) {
                flush_chunk(
                    &mut segments,
                    &mut chunk_start_idx,
                    &mut chunk_last_non_blank,
                );
                segments.push(attached_verbatim(start, end));
                idx = end + 1;
                continue;
            }
        }

        // Where the current top-level block starts, so a block can be kept whole below.
        if is_root
            && !line.is_blank
            && (idx == 0 || lines[idx - 1].is_blank)
            && !line.raw.starts_with([' ', '\t'])
            && !hard_protected.overlaps(line.start, line.start)
        {
            block_start = idx;
        }

        // Commit-message comment lines (`# ...`, `; ...`, `JJ: ...`) at column 0.
        if let Some(comment_prefix) = commit.map(|c| c.comment_prefix).filter(|_| is_root) {
            if !line.is_blank && is_commit_comment_line(line.raw, comment_prefix) {
                let mut end_idx = idx;
                while lines.get(end_idx + 1).is_some_and(|next| {
                    !next.is_blank && is_commit_comment_line(next.raw, comment_prefix)
                }) {
                    end_idx += 1;
                }
                let attached = (idx > 0 && !lines[idx - 1].is_blank)
                    || lines.get(end_idx + 1).is_some_and(|next| !next.is_blank);
                if attached {
                    // CommonMark reads the comment as part of the text around it (a paragraph,
                    // list item, or code span), while Git strips it and joins what remains. No
                    // reflow is safe for both readings, so keep the whole block as written.
                    let block_first = rewind_to_block(
                        &mut segments,
                        &mut chunk_start_idx,
                        &mut chunk_last_non_blank,
                        &lines,
                        block_start,
                        idx,
                    );
                    let mut block_last = commit_block_end(&lines, end_idx, &hard_protected);
                    // An armored block (a PGP signature) that starts in it is kept whole too,
                    // through the CommonMark block attached after its closing line.
                    let mut i = end_idx + 1;
                    while i <= block_last {
                        if let Some(close_idx) = match_armored_block(&lines, i) {
                            let limit = if close_idx < trailer_search_end
                                && trailer_search_end < lines.len()
                            {
                                lines[trailer_search_end].start
                            } else {
                                text.len()
                            };
                            let armor_end = extend_armored_block_end(
                                text,
                                &lines,
                                i,
                                close_idx,
                                limit,
                                comment_prefix,
                                &hard_protected,
                            );
                            block_last = block_last.max(armor_end);
                            i = armor_end;
                        }
                        i += 1;
                    }
                    segments.push(attached_verbatim(block_first, block_last));
                    idx = block_last + 1;
                    continue;
                }
                if !hard_protected.overlaps(line.start, line.start) {
                    // A comment between blank lines stays as it is without CommonMark turning
                    // `# ...` into an ATX heading.
                    flush_chunk(
                        &mut segments,
                        &mut chunk_start_idx,
                        &mut chunk_last_non_blank,
                    );
                    segments.push(attached_verbatim(idx, end_idx));
                    idx = end_idx + 1;
                    // Indented lines after the comment continue a list item or quote opened
                    // above it (Git strips the comment and rejoins them). Formatted on their own
                    // they would lose that indentation, so keep them as they are.
                    if let Some(run_end) = indented_run_end(&lines, idx) {
                        let run_start = (idx..=run_end)
                            .find(|&i| !lines[i].is_blank)
                            .unwrap_or(run_end);
                        segments.push(attached_verbatim(run_start, run_end));
                        idx = run_end + 1;
                    }
                    continue;
                }
            }
        }

        // A `---` patch divider line stays as it is: below a paragraph CommonMark reads it as a
        // setext underline, and widening it would move the end of Git's trailer search (the
        // first one) or change the message text Git keeps (any one). Inside a code span or other
        // literal it is literal text, which reflow keeps at a line start.
        if commit.is_some()
            && is_root
            && is_git_patch_divider(line.raw)
            && !hard_protected.overlaps(line.start, line.start)
        {
            flush_chunk(
                &mut segments,
                &mut chunk_start_idx,
                &mut chunk_last_non_blank,
            );
            segments.push(attached_verbatim(idx, idx));
            idx += 1;
            continue;
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

            // Commit-message armored signatures and trailers (root level only).
            if let Some(commit) = commit.filter(|_| is_root) {
                if let Some(close_idx) = match_armored_block(&lines, idx) {
                    let starts_armor = starts_block
                        || chunk_start_idx.is_some_and(|chunk_start| {
                            starts_top_level_paragraph(
                                &text[lines[chunk_start].start..],
                                line.start - lines[chunk_start].start,
                                content_start - lines[chunk_start].start,
                            )
                        });
                    if starts_armor {
                        let limit =
                            if close_idx < trailer_search_end && trailer_search_end < lines.len() {
                                lines[trailer_search_end].start
                            } else {
                                text.len()
                            };
                        let end_idx = extend_armored_block_end(
                            text,
                            &lines,
                            idx,
                            close_idx,
                            limit,
                            commit.comment_prefix,
                            &hard_protected,
                        );
                        flush_chunk(
                            &mut segments,
                            &mut chunk_start_idx,
                            &mut chunk_last_non_blank,
                        );
                        segments.push(attached_verbatim(idx, end_idx));
                        idx = end_idx + 1;
                        continue;
                    }
                }
                if starts_block {
                    if let Some(end_idx) =
                        match_commit_trailer_block(&lines, idx, commit, trailer_search_end)
                    {
                        flush_chunk(
                            &mut segments,
                            &mut chunk_start_idx,
                            &mut chunk_last_non_blank,
                        );
                        segments.push(attached_verbatim(idx, end_idx));
                        idx = end_idx + 1;
                        continue;
                    }
                } else if chunk_last_non_blank == Some(idx - 1)
                    && !ends_with_markdown_hard_break(lines[idx - 1].raw)
                {
                    if let Some(end_idx) = match_attached_trailer_suffix(
                        &lines,
                        idx,
                        commit,
                        trailer_search_end,
                        &hard_protected,
                    ) {
                        flush_chunk(
                            &mut segments,
                            &mut chunk_start_idx,
                            &mut chunk_last_non_blank,
                        );
                        segments.push(attached_verbatim(idx, end_idx));
                        idx = end_idx + 1;
                        continue;
                    }
                }
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
                let is_col0_standalone = |l: &LineSpan<'_>| {
                    !l.is_blank
                        && !l.raw.starts_with([' ', '\t'])
                        && (is_standalone_prompt_xml_tag(l.raw.trim())
                            || is_standalone_at_import(l.raw.trim()))
                };
                if let Some((b_range, kind)) = find_top_block(&top_blocks, line.start) {
                    let b_start = lines.partition_point(|l| l.end_excl <= b_range.start);
                    let b_end = (b_start..lines.len())
                        .take_while(|&i| lines[i].start < b_range.end)
                        .filter(|&i| !lines[i].is_blank && !is_col0_standalone(&lines[i]))
                        .last();
                    if kind == TopBlockKind::ContextSensitive {
                        // A column-0 prompt tag in the middle of a list, blockquote, footnote, or
                        // setext heading would hide the surrounding container/heading context from
                        // the lines that follow it (such as `  ---` or `  - nested`), so keep the
                        // enclosing block intact up to its trailing standalone tags.
                        if let Some(mut end_i) = b_end.filter(|&end_i| b_start < idx && end_i > idx)
                        {
                            if let Some(comment_prefix) =
                                commit.map(|c| c.comment_prefix).filter(|_| is_root)
                            {
                                if let Some(c_idx) = (b_start..=end_i).rfind(|&i| {
                                    !lines[i].is_blank
                                        && is_commit_comment_line(lines[i].raw, comment_prefix)
                                }) {
                                    end_i =
                                        end_i.max(commit_block_end(&lines, c_idx, &hard_protected));
                                }
                            }
                            let first_i = rewind_to_block(
                                &mut segments,
                                &mut chunk_start_idx,
                                &mut chunk_last_non_blank,
                                &lines,
                                b_start,
                                idx,
                            );
                            segments.push(attached_verbatim(first_i, end_i));
                            idx = end_i + 1;
                            continue;
                        }
                    } else if let Some(mut end_i) =
                        b_end.filter(|&end_i| b_start < idx && end_i > idx)
                    {
                        let next_idx = idx + 1;
                        if next_idx < lines.len()
                            && !lines[next_idx].is_blank
                            && !is_col0_standalone(&lines[next_idx])
                        {
                            let (_, next_lb) = leading_ws_columns(lines[next_idx].raw, TAB_STOP);
                            if !starts_top_level_paragraph(
                                &text[lines[next_idx].start..],
                                0,
                                next_lb,
                            ) {
                                flush_chunk(
                                    &mut segments,
                                    &mut chunk_start_idx,
                                    &mut chunk_last_non_blank,
                                );
                                let blank_before = idx > 0 && lines[idx - 1].is_blank;
                                segments.push(PreSegment::StandaloneLine {
                                    text: trimmed.to_string(),
                                    blank_before,
                                    blank_after: false,
                                    range: line.start..line.end_excl,
                                    start_line: line.line_no,
                                    end_line: line.line_no,
                                });
                                if let Some(comment_prefix) =
                                    commit.map(|c| c.comment_prefix).filter(|_| is_root)
                                {
                                    if let Some(c_idx) = (next_idx..=end_i).rfind(|&i| {
                                        !lines[i].is_blank
                                            && is_commit_comment_line(lines[i].raw, comment_prefix)
                                    }) {
                                        end_i = end_i.max(commit_block_end(
                                            &lines,
                                            c_idx,
                                            &hard_protected,
                                        ));
                                    }
                                }
                                segments.push(attached_verbatim(next_idx, end_i));
                                idx = end_i + 1;
                                continue;
                            }
                        }
                    }
                }
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

/// The last line of root-level frontmatter in `lines` (never a single-line commit subject).
fn root_frontmatter_end(lines: &[LineSpan<'_>], commit: Option<CommitSyntax<'_>>) -> Option<usize> {
    if commit.is_some_and(|c| is_single_line_commit_subject_at(lines, 0, c.comment_prefix)) {
        return None;
    }
    match_frontmatter(lines)
}

/// Whether root-level `text` starts with frontmatter that [`preprocess`] would keep aside.
pub(crate) fn starts_with_root_frontmatter(text: &str, commit: Option<CommitSyntax<'_>>) -> bool {
    root_frontmatter_end(&split_line_spans(text), commit).is_some()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TopBlockKind {
    /// `List`, `BlockQuote`, `FootnoteDefinition`, or `Heading`: splitting inside it hides
    /// container indentation or heading underline context from later lines.
    ContextSensitive,
    /// Top-level `Paragraph`: safe to split at a standalone line unless the remainder would parse
    /// as a different block construct in isolation.
    Paragraph,
}

fn find_top_block(
    blocks: &[(Range<usize>, TopBlockKind)],
    offset: usize,
) -> Option<(Range<usize>, TopBlockKind)> {
    let idx = blocks.partition_point(|(r, _)| r.end <= offset);
    blocks
        .get(idx)
        .filter(|(r, _)| r.start <= offset)
        .map(|(r, kind)| (r.clone(), *kind))
}

fn rewind_to_block(
    segments: &mut Vec<PreSegment>,
    chunk_start_idx: &mut Option<usize>,
    chunk_last_non_blank: &mut Option<usize>,
    lines: &[LineSpan<'_>],
    mut block_first: usize,
    idx: usize,
) -> usize {
    match *chunk_start_idx {
        Some(chunk_start) if chunk_start < block_first => {
            // The chunk holds earlier blocks too: emit just those.
            *chunk_last_non_blank = (chunk_start..block_first)
                .rev()
                .find(|&i| !lines[i].is_blank);
            if let (Some(start_i), Some(end_i)) = (*chunk_start_idx, *chunk_last_non_blank) {
                if start_i <= end_i {
                    segments.push(PreSegment::MarkdownChunk {
                        range: lines[start_i].start..lines[end_i].end_excl,
                        start_line: lines[start_i].line_no,
                    });
                }
            }
            *chunk_start_idx = None;
            *chunk_last_non_blank = None;
        }
        _ => {
            // Pieces of this block may already be emitted (a standalone prompt
            // tag line splits a paragraph): they join the kept block.
            *chunk_start_idx = None;
            *chunk_last_non_blank = None;
            let block_offset = lines[block_first].start;
            while let Some(range) = segments.last().map(PreSegment::range) {
                if range.start >= block_offset {
                    segments.pop();
                    continue;
                }
                if range.end > block_offset {
                    if let Some(PreSegment::MarkdownChunk { range, .. }) = segments.last_mut() {
                        // A chunk holding earlier blocks and the start of this
                        // one: cut it at the block boundary.
                        let chunk_start = range.start;
                        match (0..block_first)
                            .rev()
                            .find(|&i| !lines[i].is_blank && lines[i].start >= chunk_start)
                        {
                            Some(last) => range.end = lines[last].end_excl,
                            None => {
                                segments.pop();
                            }
                        }
                    } else {
                        // Other segments are kept as written or are single
                        // constructs; the kept block starts after them.
                        block_first = (block_first..=idx)
                            .find(|&i| lines[i].start >= range.end)
                            .unwrap_or(idx);
                    }
                }
                break;
            }
        }
    }
    block_first
}

/// Ranges where preprocessing must not recognize extensions, as `(hard, soft)`, shifted by
/// `offset`. Hard ranges are literals and opaque HTML. Soft ranges are containers and inline
/// HTML, where a standalone prompt tag at column 0 is still recognized.
fn protected_ranges(text: &str) -> (Intervals, Intervals) {
    let (hard, soft, _) = protected_ranges_from(text, 0);
    (hard, soft)
}

fn protected_ranges_from(
    text: &str,
    offset: usize,
) -> (Intervals, Intervals, Vec<(Range<usize>, TopBlockKind)>) {
    let shift = |r: Range<usize>| (r.start + offset)..(r.end + offset);
    let mut hard = Vec::new();
    let mut soft = Vec::new();
    let mut top_blocks = Vec::new();
    let mut depth = 0usize;
    for (event, range) in Parser::new_ext(text, parser_options()).into_offset_iter() {
        if let Event::Start(tag) = &event {
            if depth == 0 {
                match tag {
                    Tag::List(_)
                    | Tag::BlockQuote(_)
                    | Tag::FootnoteDefinition(_)
                    | Tag::Heading { .. } => {
                        top_blocks.push((shift(range.clone()), TopBlockKind::ContextSensitive));
                    }
                    Tag::Paragraph => {
                        top_blocks.push((shift(range.clone()), TopBlockKind::Paragraph));
                    }
                    _ => {}
                }
            }
            depth += 1;
        } else if matches!(event, Event::End(_)) {
            depth = depth.saturating_sub(1);
        }
        match event {
            Event::Start(Tag::CodeBlock(_))
            | Event::Code(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => hard.push(shift(range)),
            Event::Start(Tag::HtmlBlock) => {
                let first = text[range.clone()].lines().next().unwrap_or("").trim();
                if !is_standalone_prompt_xml_tag(first)
                    && !is_formatter_off_comment(first)
                    && !is_ignore_next_comment(first)
                {
                    hard.push(shift(range));
                }
            }
            Event::Start(Tag::List(_) | Tag::BlockQuote(_) | Tag::FootnoteDefinition(_))
            | Event::InlineHtml(_) => soft.push(shift(range)),
            _ => {}
        }
    }
    (Intervals::new(hard), Intervals::new(soft), top_blocks)
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

/// The Git settings that decide how a commit message's lines are read: the comment prefix
/// (`core.commentChar` / `core.commentString`, resolved from `auto`) and the trailer separator
/// characters (`trailer.separators`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommitSyntax<'a> {
    pub comment_prefix: &'a str,
    pub trailer_separators: &'a str,
    /// Names and `key` values of Git's configured `trailer.<name>.*` trailers.
    pub trailer_keys: &'a [String],
}

/// Whether `raw` is the `---` line where Git's trailer parsing (`trailer.c:find_patch_start`)
/// takes the patch to begin: `---` followed by whitespace or the end of the line.
pub(crate) fn is_git_patch_divider(raw: &str) -> bool {
    raw.strip_prefix("---")
        .is_some_and(|rest| rest.chars().next().is_none_or(char::is_whitespace))
}

/// Whether `git am` (`mailinfo.c:patchbreak`) reads `raw` as the start of a patch, which ends the
/// commit message: `diff -` or `Index: ` at column 0.
pub(crate) fn is_git_patch_break(raw: &str) -> bool {
    raw.starts_with("diff -") || raw.starts_with("Index: ")
}

/// Byte offset where Git stops looking for trailers in `text`: the start of the first
/// [`is_git_patch_divider`] line, or the end of `text`.
pub(crate) fn git_trailer_search_end(text: &str) -> usize {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if is_git_patch_divider(line.trim_end_matches(['\r', '\n'])) {
            return offset;
        }
        offset += line.len();
    }
    text.len()
}

/// Candidate comment characters tried by Git when `core.commentChar = auto`
/// (`builtin/commit.c:adjust_comment_line_char`).
const AUTO_COMMENT_CANDIDATES: &[&str] = &["#", ";", "@", "!", "$", "%", "^", "&", "|", ":"];

const GIT_SCISSORS_SUFFIX: &str = " ------------------------ >8 ------------------------";

/// Resolves `configured` (`FormattingOptions::comment_char`) into a concrete line prefix for
/// `text`. When `configured` is `"auto"`, detects the character Git wrote into the commit buffer's
/// status/scissors section, or picks the first unused candidate from `#;@!$%^&|:`.
pub(crate) fn resolve_comment_prefix<'a>(text: &str, configured: &'a str) -> &'a str {
    if !configured.eq_ignore_ascii_case("auto") {
        return if configured.is_empty() {
            "#"
        } else {
            configured
        };
    }

    let raw_lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    for &c in AUTO_COMMENT_CANDIDATES {
        if raw_lines
            .iter()
            .any(|l| l.strip_prefix(c) == Some(GIT_SCISSORS_SUFFIX))
        {
            return c;
        }
    }
    for &c in AUTO_COMMENT_CANDIDATES {
        let hint = format!("with '{c}' will be ignored");
        if raw_lines
            .iter()
            .any(|l| l.starts_with(c) && l.contains(&hint))
        {
            return c;
        }
    }
    // Check for a trailing status block of 2+ consecutive lines starting with the same candidate.
    let trailing_non_blank: Vec<&str> = raw_lines
        .iter()
        .copied()
        .rev()
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty())
        .collect();
    if trailing_non_blank.len() >= 2 {
        for &c in AUTO_COMMENT_CANDIDATES {
            if trailing_non_blank.iter().all(|l| l.starts_with(c)) {
                return c;
            }
        }
    }
    for &c in AUTO_COMMENT_CANDIDATES {
        if !raw_lines.iter().any(|l| l.starts_with(c)) {
            return c;
        }
    }
    "#"
}

/// Whether `raw` is a Git or Jujutsu commit comment line at column 0.
pub(crate) fn is_commit_comment_line(raw: &str, comment_prefix: &str) -> bool {
    let bare = comment_prefix.trim_end_matches(' ');
    raw.starts_with(comment_prefix) || (!bare.is_empty() && raw == bare) || raw.starts_with("JJ:")
}

/// Whether `raw` is an exact Git (`<comment_prefix> ------------------------ >8 ------------------------`)
/// or Jujutsu (`JJ: ignore-rest`) scissors line.
///
/// Git writes the prefix exactly as configured followed by its own space, so a prefix that ends
/// in a space (`core.commentString = "// "`) is followed by two spaces.
pub(crate) fn is_commit_scissors_line(raw: &str, comment_prefix: &str) -> bool {
    raw == "JJ: ignore-rest"
        || (!comment_prefix.trim().is_empty()
            && raw.strip_prefix(comment_prefix) == Some(GIT_SCISSORS_SUFFIX))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitTailSplit {
    /// Byte end of the active commit message head (excluding blank lines immediately before tail).
    pub head_end: usize,
    /// Verbatim tail range in `text` (omitting at most one final document `\n`), if present.
    pub tail_range: Option<Range<usize>>,
    pub tail_start_line: usize,
    pub tail_end_line: usize,
    pub had_blank_before_tail: bool,
}

/// Splits off any trailing comment block and/or scissors diff section (`git commit -v`,
/// `JJ: ignore-rest`) at the end of LF-normalized `text`.
pub(crate) fn split_commit_tail(text: &str, comment_prefix: &str) -> CommitTailSplit {
    let lines = split_line_spans(text);
    if lines.is_empty() {
        return CommitTailSplit {
            head_end: 0,
            tail_range: None,
            tail_start_line: 1,
            tail_end_line: 1,
            had_blank_before_tail: false,
        };
    }

    let scissors_idx = lines
        .iter()
        .position(|l| is_commit_scissors_line(l.raw, comment_prefix));
    let mut tail_idx = scissors_idx.unwrap_or(lines.len());

    // A `#` line that closes a multi-line code span (or sits in other literal text) is part of
    // that literal for CommonMark, so it stays with the message it belongs to.
    let message_end = lines.get(tail_idx).map_or(text.len(), |l| l.start);
    let mut literals: Option<Intervals> = None;
    while let Some(prev_non_blank) = (0..tail_idx).rev().find(|&i| !lines[i].is_blank) {
        let line = lines[prev_non_blank];
        if !is_commit_comment_line(line.raw, comment_prefix) {
            break;
        }
        let literals = literals.get_or_insert_with(|| protected_ranges(&text[..message_end]).0);
        if literals.overlaps(line.start, line.start) {
            break;
        }
        tail_idx = prev_non_blank;
    }

    if tail_idx >= lines.len() {
        return CommitTailSplit {
            head_end: text.len(),
            tail_range: None,
            tail_start_line: lines.len(),
            tail_end_line: lines.len(),
            had_blank_before_tail: false,
        };
    }

    let last_head_idx = (0..tail_idx).rev().find(|&i| !lines[i].is_blank);
    let had_blank_before_tail = last_head_idx.is_some_and(|h| h + 1 < tail_idx);
    let head_end = last_head_idx.map_or(0, |h| (lines[h].end_excl + 1).min(lines[tail_idx].start));
    let tail_start = lines[tail_idx].start;
    let tail_end = text
        .strip_suffix('\n')
        .map_or(text.len(), str::len)
        .max(tail_start);

    CommitTailSplit {
        head_end,
        tail_range: Some(tail_start..tail_end),
        tail_start_line: lines[tail_idx].line_no,
        tail_end_line: lines[lines.len() - 1].line_no,
        had_blank_before_tail,
    }
}

/// The last line of the top-level block that continues past line `from`: through following
/// non-blank lines, across blank lines followed by indented ones (list item and quote
/// continuations), and through fenced code and other literals that contain blank lines.
fn commit_block_end(lines: &[LineSpan<'_>], from: usize, hard_protected: &Intervals) -> usize {
    let mut last = from;
    let mut fence: Option<FenceState> = None;
    let mut idx = from + 1;
    while idx < lines.len() {
        let line = lines[idx];
        if let Some(open) = fence {
            if is_fence_close(line.raw, open.ch, open.count) {
                fence = None;
            }
            last = idx;
            idx += 1;
            continue;
        }
        if line.is_blank {
            // Blank lines inside an HTML block or other literal (even trailing ones, as in an
            // unclosed `<pre>`) belong to it.
            if hard_protected.overlaps(line.start, line.start) {
                last = idx;
                idx += 1;
                continue;
            }
            match (idx..lines.len()).find(|&i| !lines[i].is_blank) {
                Some(next) if lines[next].raw.starts_with([' ', '\t']) => {
                    idx = next;
                    continue;
                }
                _ => break,
            }
        }
        fence = parse_fence_open(line.raw.trim_start());
        last = idx;
        idx += 1;
    }
    last
}

/// If the first non-blank line at or after `start` is indented, returns the index of the last
/// non-blank line of the run of indented and blank lines starting there (including any fenced
/// code block opened inside it, wherever its closing fence is).
fn indented_run_end(lines: &[LineSpan<'_>], start: usize) -> Option<usize> {
    let first = (start..lines.len()).find(|&i| !lines[i].is_blank)?;
    if !lines[first].raw.starts_with([' ', '\t']) {
        return None;
    }
    let mut last = first;
    let mut idx = first;
    while idx < lines.len() {
        let line = lines[idx];
        if line.is_blank {
            idx += 1;
            continue;
        }
        if !line.raw.starts_with([' ', '\t']) {
            break;
        }
        if let Some(open) = parse_fence_open(line.raw.trim_start()) {
            idx += 1;
            while idx < lines.len() && !is_fence_close(lines[idx].raw, open.ch, open.count) {
                idx += 1;
            }
            idx = idx.min(lines.len() - 1);
        }
        last = idx;
        idx += 1;
    }
    Some(last)
}

fn is_single_line_commit_subject_at(
    lines: &[LineSpan<'_>],
    idx: usize,
    comment_prefix: &str,
) -> bool {
    let first = lines[idx];
    // A complete armored block (a lone PGP signature) is kept whole by the armor rule instead.
    if first.is_blank
        || is_commit_comment_line(first.raw, comment_prefix)
        || match_armored_block(lines, idx).is_some()
    {
        return false;
    }
    let (_, lead_bytes) = leading_ws_columns(first.raw, TAB_STOP);
    if lead_bytes <= 3
        && !first.raw.starts_with('\t')
        && parse_fence_open(&first.raw[lead_bytes..]).is_some()
    {
        return false;
    }
    lines
        .get(idx + 1)
        .is_none_or(|next| next.is_blank || is_commit_comment_line(next.raw, comment_prefix))
}

fn ends_with_markdown_hard_break(raw: &str) -> bool {
    let trimmed_end = raw.trim_end_matches([' ', '\t']);
    if trimmed_end.is_empty() {
        return false;
    }
    let backslashes = trimmed_end
        .bytes()
        .rev()
        .take_while(|&b| b == b'\\')
        .count();
    (trimmed_end.ends_with('\\') && backslashes % 2 == 1) || raw.ends_with("  ")
}

fn match_armored_block(lines: &[LineSpan<'_>], start: usize) -> Option<usize> {
    let open = lines[start].raw.trim();
    let suffix = open.strip_prefix("-----BEGIN ")?;
    let label = suffix.strip_suffix("-----")?;
    if label.is_empty() || label.contains('-') {
        return None;
    }
    let expected_close = format!("-----END {label}-----");
    for (idx, line) in lines.iter().enumerate().skip(start + 1) {
        if line.raw.trim() == expected_close {
            return Some(idx);
        }
    }
    None
}

/// Whether the line at `rel_line_start..=rel_content_start` starts a new top-level paragraph or
/// heading inside `chunk_to_end` (for instance immediately after a closing code fence, rule, or
/// ATX heading without a blank line), rather than continuing a preceding paragraph or table.
fn starts_top_level_paragraph(
    chunk_to_end: &str,
    rel_line_start: usize,
    rel_content_start: usize,
) -> bool {
    let mut depth = 0usize;
    for (event, range) in Parser::new_ext(chunk_to_end, parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Paragraph | Tag::Heading { .. }) if depth == 0 => {
                if (rel_line_start..=rel_content_start).contains(&range.start) {
                    return true;
                }
                if range.start > rel_content_start {
                    return false;
                }
                depth += 1;
            }
            Event::Start(_) => {
                if depth == 0 && range.start > rel_content_start {
                    return false;
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// Extends an armored block ending at `close_idx` across any immediately attached continuation
/// lines of the same CommonMark block (so lines like `    continuation\nfollowing text` attached
/// directly to `-----END ...-----` are not detached and reparsed as indented code).
fn extend_armored_block_end(
    text: &str,
    lines: &[LineSpan<'_>],
    start_idx: usize,
    close_idx: usize,
    limit: usize,
    comment_prefix: &str,
    hard_protected: &Intervals,
) -> usize {
    if close_idx + 1 >= lines.len()
        || lines[close_idx + 1].is_blank
        || lines[close_idx + 1].start >= limit
    {
        return close_idx;
    }
    let base = lines[start_idx].start;
    let rel_close = lines[close_idx].start - base;
    let rel_close_end = lines[close_idx]
        .end_excl
        .max(lines[close_idx].start + 1)
        .saturating_sub(base);
    let slice = &text[base..limit];
    let mut depth = 0usize;
    let mut top_start = 0usize;
    let mut abs_end = lines[close_idx].end_excl;
    for (event, range) in Parser::new_ext(slice, parser_options()).into_offset_iter() {
        match event {
            Event::Start(_) => {
                if depth == 0 {
                    top_start = range.start;
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 && top_start < rel_close_end && range.end > rel_close {
                    abs_end = base + range.end;
                    break;
                }
            }
            _ => {}
        }
    }
    let mut end_idx = (close_idx..lines.len())
        .take_while(|&i| lines[i].start < abs_end)
        .filter(|&i| !lines[i].is_blank)
        .last()
        .unwrap_or(close_idx);
    if let Some(comment_idx) = ((close_idx + 1)..=end_idx)
        .find(|&i| !lines[i].is_blank && is_commit_comment_line(lines[i].raw, comment_prefix))
    {
        end_idx = end_idx.max(commit_block_end(lines, comment_idx, hard_protected));
    }
    end_idx
}

/// Whether `word` is a valid Git trailer token name (`[A-Za-z0-9-]+`).
pub(crate) fn is_trailer_token_name(word: &str) -> bool {
    !word.is_empty() && word.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Whether `word` at the start of a line would make it a trailer line: `Token:` (a word ending
/// right after the `:`, so URLs are not included) or, for another configured separator such as
/// `=`, `Token=` followed by anything.
pub(crate) fn is_trailer_token_word(word: &str, separators: &str) -> bool {
    let Some(pos) = word.find(|c| separators.contains(c)) else {
        return false;
    };
    is_trailer_token_name(&word[..pos]) && (!word[pos..].starts_with(':') || pos + 1 == word.len())
}

/// Matches Git's `trailer.c:find_separator` for the `trailer.separators` characters, returning
/// the token before optional horizontal whitespace and the separator's byte offset.
pub(crate) fn find_git_trailer_separator<'a>(
    line: &'a str,
    separators: &str,
) -> Option<(&'a str, usize)> {
    if line.is_empty() || line.starts_with([' ', '\t']) {
        return None;
    }
    let sep = line.find(|c| separators.contains(c))?;
    let token = line[..sep].trim_end_matches([' ', '\t']);
    is_trailer_token_name(token).then_some((token, sep))
}

/// The text after the separator character starting at byte `sep`.
fn after_separator(line: &str, sep: usize) -> &str {
    let mut rest = line[sep..].chars();
    rest.next();
    rest.as_str()
}

/// The value after the separator at `sep`, unless the separator is a `:` glued to the value
/// (`https://...`), which reads as prose rather than a trailer.
fn trailer_value(line: &str, sep: usize) -> Option<&str> {
    let after = after_separator(line, sep);
    let glued_colon = line[sep..].starts_with(':') && !after.is_empty();
    (!glued_colon || after.starts_with([' ', '\t'])).then_some(after)
}

fn is_git_generated_prefix(line: &str) -> bool {
    line.starts_with("Signed-off-by: ") || line.starts_with("(cherry picked from commit ")
}

/// Whether `line` is an unambiguous Git/Gerrit trailer line (`Change-Id:`, `Signed-off-by:`,
/// `Bug:`, `Test:`, `Relnote:`, `(cherry picked from commit ...)`, etc.) rather than a URL or
/// generic prose label like `Note:`.
pub(crate) fn is_well_known_trailer_line(line: &str, separators: &str) -> bool {
    if line.starts_with("(cherry picked from commit ") {
        return true;
    }
    let Some((token, sep)) = find_git_trailer_separator(line, separators) else {
        return false;
    };
    if trailer_value(line, sep).is_none() {
        return false;
    }
    if token.contains('-') && token.as_bytes()[0].is_ascii_alphabetic() {
        return true;
    }
    matches!(
        token.to_ascii_lowercase().as_str(),
        "bug"
            | "bugs"
            | "fix"
            | "fixes"
            | "fixed"
            | "close"
            | "closes"
            | "closed"
            | "resolve"
            | "resolves"
            | "resolved"
            | "ref"
            | "refs"
            | "reference"
            | "references"
            | "issue"
            | "issues"
            | "test"
            | "tests"
            | "testing"
            | "relnote"
            | "flag"
            | "cc"
            | "bcc"
            | "link"
            | "see"
            | "acked"
    )
}

/// The lines Git's `trailer.c` reads as a commit message's trailer block candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitTrailerCandidate {
    /// Indices of the candidate lines: from just after the blank line where Git's backward scan
    /// stopped to the end of the log message (excluding the trailing comment and empty lines that
    /// Git ignores). Comment lines inside the range are skipped by Git but still part of it.
    pub lines: Range<usize>,
    /// Whether Git accepts the candidate as a trailer block (`git interpret-trailers --parse`
    /// prints it).
    pub is_block: bool,
}

/// C `isspace` on a byte (Git's `is_blank_line` and continuation checks).
fn is_c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Mirrors Git's `trailer.c` (`find_end_of_log_message`, `ignored_log_message_bytes`, and
/// `find_trailer_block_start`) on a whole LF-split commit message: stops at the first `---`
/// patch divider and any scissors line, ignores the run of comment and empty lines that ends
/// the log message, skips the title paragraph, and scans backward to the last blank line,
/// skipping comment lines. Returns `None` when Git finds no blank-line-separated candidate.
pub(crate) fn find_git_trailer_candidate(
    raw_lines: &[&str],
    commit: CommitSyntax<'_>,
) -> Option<GitTrailerCandidate> {
    let prefix = commit.comment_prefix;
    let is_comment = |line: &str| is_git_comment_line(line, prefix);
    let is_blank = |line: &str| line.bytes().all(is_c_space);
    let end = git_log_message_end(raw_lines, commit).end;

    // The first paragraph is the title and cannot hold trailers.
    let title_end = (0..end).find(|&i| !is_comment(raw_lines[i]) && is_blank(raw_lines[i]))?;

    let mut only_spaces = true;
    let mut trailer_lines = 0usize;
    let mut non_trailer_lines = 0usize;
    let mut possible_continuation_lines = 0usize;
    let mut recognized_prefix = false;
    for i in (title_end..end).rev() {
        let line = raw_lines[i];
        if is_comment(line) {
            non_trailer_lines += possible_continuation_lines;
            possible_continuation_lines = 0;
            continue;
        }
        if is_blank(line) {
            if only_spaces {
                continue;
            }
            non_trailer_lines += possible_continuation_lines;
            let is_block = trailer_lines > 0
                && (non_trailer_lines == 0
                    || (recognized_prefix && trailer_lines * 3 >= non_trailer_lines));
            return Some(GitTrailerCandidate {
                lines: i + 1..end,
                is_block,
            });
        }
        only_spaces = false;
        if is_git_generated_prefix(line) {
            trailer_lines += 1;
            possible_continuation_lines = 0;
            recognized_prefix = true;
        } else if let Some((_, sep)) = find_git_trailer_separator(line, commit.trailer_separators) {
            trailer_lines += 1;
            possible_continuation_lines = 0;
            // `trailer.c:token_matches_item`: the text before the separator is a prefix of a
            // configured trailer's name or key.
            let token = &line[..sep];
            if commit.trailer_keys.iter().any(|key| {
                key.get(..token.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(token))
            }) {
                recognized_prefix = true;
            }
        } else if line.bytes().next().is_some_and(is_c_space) {
            possible_continuation_lines += 1;
        } else {
            non_trailer_lines += 1 + possible_continuation_lines;
            possible_continuation_lines = 0;
        }
    }
    None
}

/// Where Git's `trailer.c` takes a commit message's log message to end.
pub(crate) struct GitLogMessageEnd {
    /// The line after the log message: the start of the final run of comment and empty lines
    /// (and old-style `Conflicts:` blocks) above any scissors line and `---` patch divider.
    pub end: usize,
    /// The line where Git stops reading (the scissors line, `---` divider, or the end).
    pub cutoff: usize,
    /// The ignored final run holds a `Conflicts:` block, which Git only skips when it is
    /// exactly `Conflicts:` followed by tab-indented paths.
    pub has_conflicts: bool,
}

/// Whether Git itself reads `raw` as a comment line (`starts_with(comment_line_str)`). Unlike
/// [`is_commit_comment_line`], this excludes Jujutsu's `JJ:` lines, which Git reads as text (or
/// trailers), so the trailer analysis below matches `git interpret-trailers`.
pub(crate) fn is_git_comment_line(raw: &str, comment_prefix: &str) -> bool {
    !comment_prefix.is_empty() && raw.starts_with(comment_prefix)
}

/// Mirrors Git's `find_end_of_log_message` and `ignored_log_message_bytes` on LF-split lines.
pub(crate) fn git_log_message_end(
    raw_lines: &[&str],
    commit: CommitSyntax<'_>,
) -> GitLogMessageEnd {
    let prefix = commit.comment_prefix;
    let divider = raw_lines
        .iter()
        .position(|l| is_git_patch_divider(l))
        .unwrap_or(raw_lines.len());
    // `wt_status_locate_end`: everything from a scissors line on is ignored.
    let cutoff = raw_lines[..divider]
        .iter()
        .position(|l| {
            !prefix.trim().is_empty() && l.strip_prefix(prefix) == Some(GIT_SCISSORS_SUFFIX)
        })
        .unwrap_or(divider);
    let mut run_start: Option<usize> = None;
    let mut in_conflicts = false;
    let mut has_conflicts = false;
    for (i, &line) in raw_lines[..cutoff].iter().enumerate() {
        if line.is_empty() || is_git_comment_line(line, prefix) {
            run_start.get_or_insert(i);
        } else if line == "Conflicts:" {
            in_conflicts = true;
            has_conflicts = true;
            run_start.get_or_insert(i);
        } else if in_conflicts && line.starts_with('\t') {
            // A path in the conflicts block.
        } else {
            run_start = None;
            in_conflicts = false;
            has_conflicts = false;
        }
    }
    GitLogMessageEnd {
        end: run_start.unwrap_or(cutoff),
        cutoff,
        has_conflicts,
    }
}

/// [`find_git_trailer_candidate`] on LF-normalized `text`.
pub(crate) fn find_git_trailer_candidate_in(
    text: &str,
    commit: CommitSyntax<'_>,
) -> Option<GitTrailerCandidate> {
    let lines: Vec<&str> = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .split('\n')
        .collect();
    find_git_trailer_candidate(&lines, commit)
}

/// Scans a contiguous non-blank paragraph starting at `start` to check whether it should be
/// preserved verbatim as a trailer block by the multi-trailer / Gerrit / multi-line quoted
/// `Relnote:` rules for squash and Gerrit messages. (The block Git itself reads as trailers is
/// found up front by [`find_git_trailer_candidate`].)
fn match_commit_trailer_block(
    lines: &[LineSpan<'_>],
    start: usize,
    commit: CommitSyntax<'_>,
    trailer_search_end: usize,
) -> Option<usize> {
    let mut end = start;
    while end + 1 < lines.len() && end + 1 != trailer_search_end && !lines[end + 1].is_blank {
        let next = lines[end + 1];
        let (cols, lb) = leading_ws_columns(next.raw, TAB_STOP);
        if cols <= 3 && !next.raw.starts_with('\t') && parse_fence_open(&next.raw[lb..]).is_some() {
            break;
        }
        end += 1;
    }

    let raw_slice: Vec<&str> = lines[start..=end].iter().map(|l| l.raw).collect();
    if is_structured_trailer_entries(&raw_slice, commit) {
        return Some(end);
    }

    None
}

/// Recognizes trailer blocks with 2+ spaced `Token: value` entries or 1+ unambiguous Git/Gerrit
/// entries (such as `Change-Id:`, `Bug:`, `Test:`, or multi-line quoted `Relnote: "..."`), even
/// when they appear mid-message in `SQUASH_MSG` or use unindented quoted `Relnote:` continuations.
pub(crate) fn is_structured_trailer_entries(raw_lines: &[&str], commit: CommitSyntax<'_>) -> bool {
    let mut entries = 0usize;
    let mut saw_well_known = false;
    let mut in_quote = false;

    for &line in raw_lines {
        if is_commit_comment_line(line, commit.comment_prefix) {
            continue;
        }
        if in_quote {
            if ends_unclosed_double_quote(line) {
                in_quote = false;
            }
            continue;
        }
        if line.starts_with("(cherry picked from commit ") {
            entries += 1;
            saw_well_known = true;
            continue;
        }
        if let Some((token, sep)) = find_git_trailer_separator(line, commit.trailer_separators) {
            let Some(after) = trailer_value(line, sep) else {
                return false;
            };
            entries += 1;
            if is_well_known_trailer_line(line, commit.trailer_separators) {
                saw_well_known = true;
                if starts_unclosed_double_quote(after.trim())
                    && token.eq_ignore_ascii_case("relnote")
                {
                    in_quote = true;
                }
            }
            continue;
        }
        if entries > 0 && line.starts_with([' ', '\t']) {
            continue;
        }
        return false;
    }

    !in_quote && (entries >= 2 || (entries == 1 && saw_well_known))
}

/// Checks if `lines[start..=end]` (at the bottom of a prose paragraph) consists solely of
/// unambiguous Git/Gerrit trailer entries so they stay on their own lines rather than merging
/// into the prose line above.
fn match_attached_trailer_suffix(
    lines: &[LineSpan<'_>],
    start: usize,
    commit: CommitSyntax<'_>,
    trailer_search_end: usize,
    hard_protected: &Intervals,
) -> Option<usize> {
    let separators = commit.trailer_separators;
    // A line continuing a multi-line code span or link is not a trailer of its own.
    if hard_protected.overlaps(lines[start].start, lines[start].start)
        || !is_well_known_trailer_line(lines[start].raw, separators)
    {
        return None;
    }
    let mut end = start;
    while end + 1 < lines.len()
        && end + 1 != trailer_search_end
        && !lines[end + 1].is_blank
        && !is_commit_comment_line(lines[end + 1].raw, commit.comment_prefix)
    {
        let next = lines[end + 1];
        if hard_protected.overlaps(next.start, next.start) {
            return None;
        }
        end += 1;
    }
    let raw_slice: Vec<&str> = lines[start..=end].iter().map(|l| l.raw).collect();
    let mut entries = 0usize;
    let mut in_quote = false;
    for &line in &raw_slice {
        if in_quote {
            if ends_unclosed_double_quote(line) {
                in_quote = false;
            }
            continue;
        }
        if is_well_known_trailer_line(line, separators) {
            entries += 1;
            if let Some((token, sep)) = find_git_trailer_separator(line, separators) {
                let after = after_separator(line, sep).trim();
                if token.eq_ignore_ascii_case("relnote") && starts_unclosed_double_quote(after) {
                    in_quote = true;
                }
            }
        } else if entries > 0 && line.starts_with([' ', '\t']) {
            continue;
        } else {
            return None;
        }
    }
    (!in_quote && entries > 0).then_some(end)
}

fn starts_unclosed_double_quote(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('"') else {
        return false;
    };
    !ends_unclosed_double_quote(rest)
}

fn ends_unclosed_double_quote(s: &str) -> bool {
    let trimmed = s.trim_end();
    if !trimmed.ends_with('"') {
        return false;
    }
    let backslashes = trimmed[..trimmed.len() - 1]
        .bytes()
        .rev()
        .take_while(|&b| b == b'\\')
        .count();
    backslashes % 2 == 0
}
