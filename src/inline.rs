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

use crate::markdown::{merged, parser_options, strictly_inside};
use crate::options::WrapMode;
use crate::preprocess::{self, CommitSyntax};
use pulldown_cmark::{Event, Parser, Tag};
use unicode_width::UnicodeWidthStr;

/// Penalty added when wrapping between words inside `[link text]`.
/// Chosen so that `(34)^2 = 1156 < LINK_BREAK_PENALTY < (35)^2 = 1225`:
/// links are kept unbroken unless breaking avoids leaving > ~34 chars of slack or overflowing.
pub const LINK_BREAK_PENALTY: i64 = 1200;

/// An unbreakable unit for line wrapping. Reflow only ever breaks lines between tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordToken {
    pub text: String,
    pub width: usize,
    pub penalty_before: i64,
    pub link_group_width: Option<usize>,
    /// The source had a line break (rather than just spaces) before this token.
    pub newline_before: bool,
}

impl WordToken {
    fn new(
        text: String,
        penalty_before: i64,
        link_group_width: Option<usize>,
        newline_before: bool,
    ) -> Self {
        let width = UnicodeWidthStr::width(text.as_str());
        Self {
            text,
            width,
            penalty_before,
            link_group_width,
            newline_before,
        }
    }
}

/// What separated an inline atom from the previous one in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sep {
    /// Directly adjacent: the atoms belong to the same word.
    None,
    Space,
    Newline,
}

/// How a hard-break-terminated paragraph segment ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardBreak {
    /// Trailing `\` before the line ending.
    Backslash,
    /// Two or more trailing spaces before the line ending (normalized to exactly two).
    Spaces,
}

impl HardBreak {
    pub fn marker(self) -> &'static str {
        match self {
            HardBreak::Backslash => "\\",
            HardBreak::Spaces => "  ",
        }
    }
}

/// A run of paragraph text between hard line breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub tokens: Vec<WordToken>,
    /// The hard break terminating this segment, or `None` for the paragraph's last segment.
    pub hard_break: Option<HardBreak>,
}

/// Returns false if placing `word` at column 0 of a wrapped line could cause CommonMark
/// or Markdeep to reinterpret the line as a heading, setext underline, list item, blockquote,
/// fence, rule, HTML block, footnote definition, display math, definition list, admonition,
/// agent import, or prompt tag.
pub fn can_start_line(word: &str) -> bool {
    if word.is_empty() {
        return true;
    }

    if (1..=6).contains(&word.len()) && word.bytes().all(|b| b == b'#') {
        return false;
    }

    // Setext `=` underline (any length, unlike `-` which is handled below with rules).
    if word.bytes().all(|b| b == b'=') {
        return false;
    }

    // HTML blocks of CommonMark types 1-6 can interrupt a paragraph (`<div>`, `<!--`, `<?`,
    // `<!DOCTYPE`, ...), and standalone tag lines are treated as prompt tags by the preprocessor.
    // Autolinks (`<https://...>`, `<a@b.c>`) are safe.
    if let Some(rest) = word.strip_prefix('<') {
        let is_tag_like = rest
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || matches!(b, b'/' | b'!' | b'?'));
        if is_tag_like && !starts_with_autolink(word) {
            return false;
        }
    }

    // Reference and footnote definitions must not be created by wrapping.
    if word.starts_with('[') && word.contains("]:") {
        return false;
    }

    // GFM table delimiter row (`|-|`, `:-:`, `|---|:-:|`) would turn the previous line into a
    // table header.
    if word.contains('-') && word.bytes().all(|b| matches!(b, b'|' | b'-' | b':')) {
        return false;
    }

    // Display math line (see `preprocess::match_display_math`).
    if word.starts_with("$$") {
        return false;
    }

    if word.starts_with('>') {
        return false;
    }

    if word.starts_with("!!!") {
        return false;
    }

    // A Markdeep diagram border (`****`, `*+**+*`) on its own line opens a diagram.
    if preprocess::is_markdeep_diagram_border(word) {
        return false;
    }

    if matches!(word, "-" | "*" | "+" | "_" | "|" | ":" | "!!!") {
        return false;
    }

    if word.len() >= 2
        && (word.bytes().all(|b| b == b'=')
            || word.bytes().all(|b| b == b'-')
            || word.bytes().all(|b| b == b'*')
            || word.bytes().all(|b| b == b'_'))
    {
        return false;
    }

    if word.starts_with("```") || word.starts_with("~~~") {
        return false;
    }

    // Ordered list marker hazard: 1..=9 digits followed by `.` or `)` (e.g. "5.", "1)")
    let bytes = word.as_bytes();
    if bytes.len() >= 2 && bytes.len() <= 10 {
        let last = bytes[bytes.len() - 1];
        if (last == b'.' || last == b')')
            && bytes[..bytes.len() - 1].iter().all(|b| b.is_ascii_digit())
        {
            return false;
        }
    }

    !(word.starts_with('@') && !word.starts_with("@["))
}

/// Like [`can_start_line`], also rejecting tokens that Git or Jujutsu commit-message handling
/// treats as comments (`<comment_prefix>...`, `JJ:...`), armored blocks (`-----BEGIN`),
/// `git am` patch breaks (`Index:`), or trailer keys (`Token:`, `(cherry`) when `commit` is set.
pub(crate) fn can_start_line_in_mode(word: &str, commit: Option<CommitSyntax<'_>>) -> bool {
    can_start_line(word) && !commit.is_some_and(|commit| is_commit_line_hazard(word, commit))
}

/// Whether `word` at column 0 would make Git or Jujutsu treat the line as a comment, armored
/// block, `git am` patch break, or trailer.
fn is_commit_line_hazard(word: &str, commit: CommitSyntax<'_>) -> bool {
    let bare_prefix = commit.comment_prefix.trim_end_matches([' ', '\t']);
    (!bare_prefix.is_empty() && word.starts_with(bare_prefix))
        || word.starts_with("JJ:")
        || word == "Index:"
        || word.starts_with("-----BEGIN")
        || word.starts_with("diff -")
        || word.eq_ignore_ascii_case("(cherry")
        || preprocess::is_trailer_token_word(word, commit.trailer_separators)
}

/// Whether `word` begins with a complete CommonMark autolink (`<scheme:...>` or
/// `<local@domain>`), which can safely start a line, unlike an HTML tag or comment.
pub(crate) fn starts_with_autolink(word: &str) -> bool {
    let Some(end) = word.find('>') else {
        return false;
    };
    let inner = &word[1..end];
    if inner.is_empty() || inner.contains(|c: char| c.is_ascii_whitespace() || c == '<') {
        return false;
    }
    if let Some((scheme, _)) = inner.split_once(':') {
        let mut chars = scheme.chars();
        if (2..=32).contains(&scheme.len())
            && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        {
            return true;
        }
    }
    let Some((local, domain)) = inner.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".!#$%&'*+/=?^_`{|}~-".contains(c))
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// Returns true if `word` placed at the end of a line would turn that line ending into a
/// hard line break (an odd number of trailing backslashes).
fn ends_with_hard_break(word: &str) -> bool {
    word.bytes().rev().take_while(|&b| b == b'\\').count() % 2 == 1
}

/// Returns true if `prefix` (the accumulated token at the start of a line) would be parsed as a
/// different block construct if a line break immediately followed it.
fn cannot_stand_alone_on_line(prefix: &str, commit_comment: Option<CommitSyntax<'_>>) -> bool {
    if !prefix.contains(' ') && !can_start_line_in_mode(prefix, commit_comment) {
        return true;
    }
    if prefix.starts_with('<')
        && !can_start_line_in_mode(prefix, commit_comment)
        && find_angle_span_end(prefix, 0) == Some(prefix.len() - 1)
    {
        return true;
    }
    for rule_char in ['_', '-', '*'] {
        if prefix.chars().all(|c| c == rule_char || c == ' ')
            && prefix.bytes().filter(|&b| b == rule_char as u8).count() >= 3
        {
            return true;
        }
    }
    // Lines the preprocessor splits out of a paragraph on the next pass (see
    // `preprocess::match_display_math` and the standalone-line rules).
    (prefix.len() >= 4 && prefix.starts_with("$$") && prefix.ends_with("$$"))
        || preprocess::is_standalone_prompt_xml_tag(prefix)
        || preprocess::is_standalone_at_import(prefix)
        || preprocess::is_markdeep_diagram_border(prefix)
}

/// Whether a line starting with `prefix` could still turn into a line that
/// [`cannot_stand_alone_on_line`] rejects as more words are added.
fn may_become_standalone_line(prefix: &str) -> bool {
    const MAX_TRACKED_BYTES: usize = 4096;
    prefix.len() <= MAX_TRACKED_BYTES && prefix.starts_with(['$', '<', '@', '*', '+', '-', '_'])
}

/// Two-word commit hazards at column 0: `diff -...` (`git am` `patchbreak()` in `mailinfo.c`) and
/// `Token :...` (`git interpret-trailers` spaced separator in `trailer.c`, or any other
/// `trailer.separators` character).
fn starts_two_word_commit_hazard(
    word: &str,
    next_word: Option<&str>,
    commit: CommitSyntax<'_>,
) -> bool {
    let Some(next) = next_word else {
        return false;
    };
    (word == "diff" && next.starts_with('-'))
        || (preprocess::is_trailer_token_name(word)
            && next.starts_with(|c| commit.trailer_separators.contains(c)))
}

/// Whether `words` joined with single spaces start with `bare_prefix` (`core.commentString`,
/// which may span multiple whitespace-separated words).
fn starts_comment_prefix<'a>(mut words: impl Iterator<Item = &'a str>, bare_prefix: &str) -> bool {
    if bare_prefix.is_empty() {
        return false;
    }
    let mut acc = String::new();
    for word in &mut words {
        if !acc.is_empty() {
            acc.push(' ');
        }
        acc.push_str(word);
        if acc.len() >= bare_prefix.len() {
            return acc.starts_with(bare_prefix);
        }
        if !bare_prefix.starts_with(&acc) {
            return false;
        }
    }
    false
}

/// Whether `words` starting at column 0 would form a multi-word comment prefix (`// comment`),
/// `diff -...` patch break, or spaced `Token :` trailer.
fn starts_commit_sequence_hazard<'a>(
    mut words: impl Iterator<Item = &'a str>,
    commit: CommitSyntax<'_>,
) -> bool {
    let Some(first) = words.next() else {
        return false;
    };
    let next = words.next();
    if starts_two_word_commit_hazard(first, next, commit) {
        return true;
    }
    let bare_prefix = commit.comment_prefix.trim_end_matches([' ', '\t']);
    starts_comment_prefix(std::iter::once(first).chain(next).chain(words), bare_prefix)
}

/// Glues tokens that must not start a line onto their predecessor, and tokens following one
/// that must not end a line onto it, so no reflow strategy can ever break between them.
fn glue_line_hazards(
    tokens: Vec<WordToken>,
    preserve: bool,
    protected_prefix_tokens: usize,
    commit_comment: Option<CommitSyntax<'_>>,
) -> Vec<WordToken> {
    let sequence_hazards: Vec<bool> = (0..tokens.len())
        .map(|i| {
            commit_comment.is_some_and(|commit| {
                let line_words = std::iter::once(&tokens[i])
                    .chain(
                        tokens[i + 1..]
                            .iter()
                            .take_while(|t| !preserve || !t.newline_before),
                    )
                    .flat_map(|t| t.text.split(' '));
                starts_commit_sequence_hazard(line_words, commit)
            })
        })
        .collect();
    // Keep every word forming an existing or newly created sequence hazard on one line.
    // Otherwise a second pass can mistake the split first line for a newly created hazard
    // and add indentation (for example `Start :u` becoming `Start\n:u`).
    let mut sequence_continuations = vec![false; tokens.len()];
    if let Some(commit) = commit_comment {
        for (start, &hazard) in sequence_hazards.iter().enumerate() {
            if !hazard {
                continue;
            }
            for end in start..tokens.len() {
                if preserve && end > start && tokens[end].newline_before {
                    break;
                }
                if starts_commit_sequence_hazard(
                    tokens[start..=end].iter().flat_map(|t| t.text.split(' ')),
                    commit,
                ) {
                    sequence_continuations[start + 1..end + 1].fill(true);
                    break;
                }
            }
        }
    }
    let mut out: Vec<WordToken> = Vec::with_capacity(tokens.len());
    // The segment so far, as it would read if the line ended here. Every segment starts a line,
    // so a break must not follow a prefix that would read as its own block (`$$a$ $b$$`,
    // `<Tag>inline text</Tag>`) even when no single token is a hazard.
    let mut line_prefix = String::new();
    let mut track_prefix = !preserve;
    for (idx, tok) in tokens.into_iter().enumerate() {
        let two_word_hazard = sequence_hazards[idx];
        let prefix_hazard = track_prefix
            && !out.is_empty()
            && cannot_stand_alone_on_line(&line_prefix, commit_comment);
        if track_prefix {
            if !line_prefix.is_empty() {
                line_prefix.push(' ');
            }
            line_prefix.push_str(&tok.text);
            track_prefix = may_become_standalone_line(&line_prefix);
        }
        if let Some(last) = out.last_mut() {
            if preserve && tok.newline_before {
                // This boundary already exists in the source. Retain its literal backslash
                // with a trailing space rather than turning a soft break into a hard break.
                if ends_with_hard_break(&last.text) {
                    last.text.push(' ');
                    last.width += 1;
                }
            } else if !can_start_line_in_mode(&tok.text, commit_comment)
                || two_word_hazard
                || sequence_continuations[idx]
                || ends_with_hard_break(&last.text)
                || idx < protected_prefix_tokens
                || cannot_stand_alone_on_line(&last.text, commit_comment)
                || prefix_hazard
            {
                last.text.push(' ');
                last.text.push_str(&tok.text);
                last.width += 1 + tok.width;
                continue;
            }
        }
        out.push(tok);
    }
    out
}

/// Which lines of a paragraph sit at column 0 of a commit message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitLevel {
    /// Inside an indented container: no segment starts at column 0.
    Nested,
    /// First paragraph of a root container with `--hanging-indent=0`: segment 0 starts after the
    /// container marker, while segments after hard breaks start at column 0.
    RootContinuation,
    /// Top-level root paragraph: every segment starts at column 0.
    Root,
}

/// Splits a raw paragraph into hard-break-separated segments and tokenizes each segment.
///
/// Hard breaks outside code, math, HTML, and link destinations split the paragraph so each run is
/// reflowed independently and re-terminated with the same style of break: a trailing `\`, or
/// two trailing spaces. Other trailing whitespace is stripped and reflowed across.
///
/// `commit_comment` is the commit-message syntax in commit-message mode. When `commit_level`
/// places a segment at column 0 of the file, a segment whose first line was indented keeps one
/// space of indentation if dedenting it would start the line with a comment prefix, trailer key,
/// or patch break. (Paragraphs with comment lines in them never get here: preprocessing keeps them
/// as written.)
pub(crate) fn tokenize_paragraph(
    raw: &str,
    mode: WrapMode,
    protect_first_line: bool,
    commit_comment: Option<CommitSyntax<'_>>,
    commit_level: CommitLevel,
) -> Vec<Segment> {
    let preserve = mode == WrapMode::Preserve;
    let raw_segments = split_hard_break_segments(raw);
    // Definition and title syntax spans hard breaks (`[a]:  \n/url`, `[label  \ntwo]: /url`),
    // so find the protected prefix on the whole paragraph, then map it onto each segment.
    let joined = raw_segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let title_end = if protect_first_line {
        let trimmed = joined.trim_start();
        trimmed
            .as_bytes()
            .first()
            .copied()
            .and_then(title_closer)
            .map_or(0, |closer| {
                title_hazard_end(&joined, joined.len() - trimmed.len() + 1, closer)
            })
    } else {
        0
    };
    let required_end = title_end.max(rejected_definition_hazard_end(&joined));
    let mut segment_start = 0usize;
    raw_segments
        .into_iter()
        .enumerate()
        .map(|(seg_idx, raw_segment)| {
            let RawSegment {
                text,
                hard_break,
                indented,
            } = raw_segment;
            let segment_required_end = required_end.saturating_sub(segment_start).min(text.len());
            segment_start += text.len() + 1;
            let mut tokens =
                tokenize_segment(&text, preserve, segment_required_end, commit_comment);
            // Preserve mode re-inserts source indentation itself.
            if let Some(commit) = commit_comment.filter(|_| !preserve) {
                match commit_level {
                    CommitLevel::Nested => {}
                    CommitLevel::RootContinuation if seg_idx == 0 => {}
                    CommitLevel::RootContinuation => {
                        keep_commit_hazard_indented(&mut tokens, commit);
                    }
                    CommitLevel::Root => {
                        if indented {
                            keep_commit_hazard_indented(&mut tokens, commit);
                        } else {
                            // Joining lines like `"//\ncomment"`, `"diff\n-u a b"`, or
                            // `"Token\n: value"`, or collapsing the whitespace in `"diff  -u"`
                            // or `"Index:\tfoo"`, would synthesize a comment line, `git am`
                            // patch break, or trailer at column 0 that was not one in the
                            // source. A first source line that is one exactly as written is left
                            // as it is.
                            let first_line = text.split('\n').next().unwrap_or("");
                            let joined = tokens
                                .iter()
                                .map(|t| t.text.as_str())
                                .collect::<Vec<_>>()
                                .join(" ");
                            let created = git_line_hazards(&joined, commit)
                                .into_iter()
                                .zip(git_line_hazards(first_line, commit))
                                .any(|(after, before)| after && !before);
                            if !tokens.is_empty() && created {
                                tokens[0].text.insert(0, ' ');
                                tokens[0].width += 1;
                            }
                        }
                    }
                }
            }
            Segment { tokens, hard_break }
        })
        .collect()
}

/// Whether `line`, exactly as written at column 0, is a Git comment (starting with the comment
/// prefix), a `git am` patch break (`mailinfo.c:patchbreak`: `diff -` or `Index: `), and a
/// trailer line (`Token: value`, `Token : value`), in that order.
fn git_line_hazards(line: &str, commit: CommitSyntax<'_>) -> [bool; 3] {
    [
        preprocess::is_commit_comment_line(line, commit.comment_prefix),
        preprocess::is_git_patch_break(line),
        preprocess::find_git_trailer_separator(line, commit.trailer_separators).is_some(),
    ]
}

/// Prefixes the first token with a space when, dedented to column 0, it would turn the line into
/// a Git comment, trailer, or patch break (CommonMark ignores the space at a line start).
fn keep_commit_hazard_indented(tokens: &mut [WordToken], commit: CommitSyntax<'_>) {
    let Some(first) = tokens.first() else {
        return;
    };
    let first_word = first.text.split(' ').next().unwrap_or("");
    let words = tokens.iter().flat_map(|t| t.text.split(' '));
    if is_commit_line_hazard(first_word, commit) || starts_commit_sequence_hazard(words, commit) {
        tokens[0].text.insert(0, ' ');
        tokens[0].width += 1;
    }
}

/// A run of paragraph source lines ending at a hard break (or the paragraph end).
struct RawSegment {
    text: String,
    hard_break: Option<HardBreak>,
    /// The segment's first source line started with whitespace.
    indented: bool,
}

/// Splits `raw` into segments separated by hard line breaks (trailing `\` or 2+ trailing spaces),
/// retaining source boundaries for preserve mode and keeping literal whitespace untouched.
fn split_hard_break_segments(raw: &str) -> Vec<RawSegment> {
    let normalized = raw.replace("\r\n", "\n");
    let raw_lines: Vec<&str> = normalized.split('\n').collect();
    let mut segments: Vec<RawSegment> = Vec::new();
    let mut current_lines: Vec<String> = Vec::new();
    let mut current_indented = false;
    let literal_spans = merged(literal_inline_ranges(&normalized));
    let mut line_start = 0usize;

    for (idx, &raw_line) in raw_lines.iter().enumerate() {
        let starts_in_literal = strictly_inside(&literal_spans, line_start);
        let trimmed_start = if starts_in_literal {
            raw_line
        } else {
            raw_line.trim_start_matches([' ', '\t'])
        };
        // Line boundaries inside code, math, and HTML are literal content.
        let line_end = line_start + raw_line.len();
        line_start = line_end + 1;
        let ends_in_literal = strictly_inside(&literal_spans, line_end);
        if current_lines.is_empty() {
            current_indented = !starts_in_literal && trimmed_start.len() < raw_line.len();
        }

        let is_last_line = idx + 1 == raw_lines.len();
        let line_no_trailing_ws = trimmed_start.trim_end_matches([' ', '\t']);
        let trailing_backslashes = line_no_trailing_ws
            .bytes()
            .rev()
            .take_while(|&b| b == b'\\')
            .count();
        let hard_break = if is_last_line || ends_in_literal || line_no_trailing_ws.is_empty() {
            None
        } else if trimmed_start.ends_with('\\') && trailing_backslashes % 2 == 1 {
            Some(HardBreak::Backslash)
        } else if trimmed_start.ends_with("  ") {
            Some(HardBreak::Spaces)
        } else {
            None
        };

        if let Some(kind) = hard_break {
            // Strip the hard-break marker and finish this segment.
            let stripped = match kind {
                HardBreak::Backslash => &line_no_trailing_ws[..line_no_trailing_ws.len() - 1],
                HardBreak::Spaces => line_no_trailing_ws,
            };
            let clean = stripped.trim_end_matches([' ', '\t']);
            if !clean.is_empty() || current_lines.is_empty() {
                current_lines.push(clean.to_string());
            }
            segments.push(RawSegment {
                text: current_lines.join("\n"),
                hard_break: Some(kind),
                indented: current_indented,
            });
            current_lines.clear();
        } else {
            let clean = if ends_in_literal {
                // Whitespace on either side of a literal newline belongs to the literal.
                trimmed_start.to_string()
            } else {
                line_no_trailing_ws.to_string()
            };
            if !clean.is_empty() || ends_in_literal || starts_in_literal {
                current_lines.push(clean);
            }
        }
    }

    if !current_lines.is_empty() {
        segments.push(RawSegment {
            text: current_lines.join("\n"),
            hard_break: None,
            indented: current_indented,
        });
    } else if let Some(last) = segments.last_mut() {
        // A hard break at the very end of a paragraph is not a break.
        last.hard_break = None;
    }

    if segments.is_empty() {
        segments.push(RawSegment {
            text: String::new(),
            hard_break: None,
            indented: false,
        });
    }

    segments
}

/// Source ranges whose whitespace is significant. Let the parser resolve precedence between
/// code, math, and HTML instead of treating backticks inside math or HTML as new code spans.
fn literal_inline_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    Parser::new_ext(text, parser_options())
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Code(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::InlineHtml(_) => Some(range),
            Event::Start(Tag::Image { .. }) => Some(range),
            Event::Start(Tag::Link { .. }) => {
                let source = &text[range.clone()];
                let bracket = if source.starts_with("![") {
                    range.start + 1
                } else if source.starts_with('[') {
                    range.start
                } else {
                    return None;
                }; // Autolinks have no bracketed label or suffix.
                find_matching_bracket(text, bracket).map(|close| close + 1..range.end)
            }
            _ => None,
        })
        .collect()
}

/// Leading indentation outside literals is restored after preserve-mode tokenization. Inside
/// literals it already belongs to the atom and must not be inserted a second time.
pub(crate) fn preserved_line_indents(text: &str) -> Vec<&str> {
    let literals = merged(literal_inline_ranges(text));
    let mut offset = 0;
    text.split('\n')
        .map(|line| {
            let inside = strictly_inside(&literals, offset);
            offset += line.len() + 1;
            if inside {
                ""
            } else {
                &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
enum InlineAtom {
    Raw {
        text: String,
        separated: Sep,
        start: usize,
    },
    LinkGroup {
        prefix: String,
        inner: String,
        suffix: String,
        separated: Sep,
        start: usize,
    },
}

impl InlineAtom {
    fn start(&self) -> usize {
        match self {
            InlineAtom::Raw { start, .. } | InlineAtom::LinkGroup { start, .. } => *start,
        }
    }

    fn separated(&self) -> Sep {
        match self {
            InlineAtom::Raw { separated, .. } | InlineAtom::LinkGroup { separated, .. } => {
                *separated
            }
        }
    }
}

fn title_closer(open: u8) -> Option<u8> {
    match open {
        b'"' => Some(b'"'),
        b'\'' => Some(b'\''),
        b'(' => Some(b')'),
        _ => None,
    }
}

fn find_unescaped_byte(bytes: &[u8], mut i: usize, target: u8) -> Option<usize> {
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == target {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn skip_ascii_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

fn skip_non_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// For a paragraph starting with a link-title delimiter (`"`, `'`, `(`) immediately after a
/// titleless reference definition, finds the byte offset past the trailing word that kept that
/// delimiter pair from being absorbed as the definition's title.
fn title_hazard_end(text: &str, scan_from: usize, closer: u8) -> usize {
    let bytes = text.as_bytes();
    // An unclosed title cannot become a title by wrapping.
    find_unescaped_byte(bytes, scan_from, closer)
        .map_or(0, |close_pos| word_end_after(bytes, close_pos + 1))
}

/// The end of the first word at or after `from` (or the end of `bytes` if there is none).
fn word_end_after(bytes: &[u8], from: usize) -> usize {
    let start = skip_ascii_ws(bytes, from);
    if start < bytes.len() {
        skip_non_ws(bytes, start)
    } else {
        bytes.len()
    }
}

/// For a paragraph starting with `[label]:` that was rejected as a link reference definition,
/// finds the byte offset past the first token that invalidated the definition so wrapping cannot
/// isolate a valid `[label]: destination` (or `[label]: destination "title"`) prefix. `text` is the
/// whole paragraph with hard breaks as newlines, since definitions can span them.
fn rejected_definition_hazard_end(text: &str) -> usize {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('[') {
        return 0;
    }
    let Some(close_bracket) = find_matching_bracket(trimmed, 0) else {
        return 0;
    };
    if trimmed.as_bytes().get(close_bracket + 1) != Some(&b':')
        || trimmed[1..close_bracket].trim().is_empty()
    {
        return 0;
    }

    let bytes = text.as_bytes();
    let lead = text.len() - trimmed.len();
    let dest_start = skip_ascii_ws(bytes, lead + close_bracket + 2);
    if dest_start >= bytes.len() {
        return 0;
    }

    let dest_end = if bytes[dest_start] == b'<' {
        let mut pos = dest_start + 1;
        let mut found = None;
        while pos < bytes.len() {
            let b = bytes[pos];
            if b == b'\\' && pos + 1 < bytes.len() {
                pos += 2;
                continue;
            }
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'<') {
                break;
            }
            if b == b'>' {
                found = Some(pos + 1);
                break;
            }
            pos += 1;
        }
        let Some(end) = found else {
            return bytes.len();
        };
        end
    } else {
        skip_non_ws(bytes, dest_start)
    };

    let after_dest = skip_ascii_ws(bytes, dest_end);
    if after_dest >= bytes.len() {
        return bytes.len();
    }

    if let Some(closer) = title_closer(bytes[after_dest]) {
        if let Some(close_pos) = find_unescaped_byte(bytes, after_dest + 1, closer) {
            return word_end_after(bytes, close_pos + 1);
        }
    }
    skip_non_ws(bytes, after_dest)
}

fn tokenize_segment(
    segment: &str,
    preserve: bool,
    required_end: usize,
    commit_comment: Option<CommitSyntax<'_>>,
) -> Vec<WordToken> {
    let mut atoms = scan_inline_atoms(segment, true);
    let k = if required_end == 0 {
        0
    } else {
        atoms
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(idx, atom)| {
                (atom.start() >= required_end && atom.separated() != Sep::None).then_some(idx)
            })
            .unwrap_or(atoms.len())
    };
    let tail_atoms = atoms.split_off(k);
    let mut tokens = atoms_to_tokens(atoms);
    let glue_count = tokens.len();
    tokens.extend(atoms_to_tokens(tail_atoms));
    glue_line_hazards(tokens, preserve, glue_count, commit_comment)
}

fn scan_inline_atoms(s: &str, allow_links: bool) -> Vec<InlineAtom> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut separated = Sep::Space;
    let mut atoms: Vec<InlineAtom> = Vec::new();
    let mut plain_buf = String::new();
    let mut plain_start = 0usize;
    // Sorted by start; `next_literal` only moves forward, keeping the scan linear.
    let mut literals = literal_inline_ranges(s);
    literals.sort_by_key(|range| range.start);
    let mut next_literal = 0usize;

    let flush_plain = |buf: &mut String, start: usize, sep: &mut Sep, out: &mut Vec<InlineAtom>| {
        if !buf.is_empty() {
            out.push(InlineAtom::Raw {
                text: std::mem::take(buf),
                separated: *sep,
                start,
            });
            *sep = Sep::None;
        }
    };

    while i < len {
        let b = bytes[i];

        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            flush_plain(&mut plain_buf, plain_start, &mut separated, &mut atoms);
            let mut saw_newline = false;
            while i < len && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                saw_newline |= bytes[i] == b'\n';
                i += 1;
            }
            separated = if saw_newline {
                Sep::Newline
            } else {
                Sep::Space
            };
            continue;
        }

        while literals
            .get(next_literal)
            .is_some_and(|range| range.start < i)
        {
            next_literal += 1;
        }
        if let Some(range) = literals
            .get(next_literal)
            .filter(|range| range.start == i)
            .cloned()
        {
            flush_plain(&mut plain_buf, plain_start, &mut separated, &mut atoms);
            // Literals (including code spans) keep their line breaks: renderers disagree on
            // the leading whitespace after a code span's line break, so it must not be joined.
            let text = s[range.clone()].to_string();
            atoms.push(InlineAtom::Raw {
                text,
                separated,
                start: i,
            });
            separated = Sep::None;
            i = range.end;
            continue;
        }

        // Backslash escape
        if b == b'\\' && i + 1 < len && bytes[i + 1].is_ascii_punctuation() {
            if plain_buf.is_empty() {
                plain_start = i;
            }
            plain_buf.push('\\');
            plain_buf.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }

        // Inline backtick code span
        if b == b'`' {
            let mut k = 1;
            while i + k < len && bytes[i + k] == b'`' {
                k += 1;
            }
            if let Some(close_end) = find_closing_backticks(s, i + k, k) {
                flush_plain(&mut plain_buf, plain_start, &mut separated, &mut atoms);
                atoms.push(InlineAtom::Raw {
                    text: s[i..close_end].to_string(),
                    separated,
                    start: i,
                });
                separated = Sep::None;
                i = close_end;
                continue;
            } else {
                if plain_buf.is_empty() {
                    plain_start = i;
                }
                for _ in 0..k {
                    plain_buf.push('`');
                }
                i += k;
                continue;
            }
        }

        // Inline math `$...$` (not `$$`)
        if b == b'$' && !s[i..].starts_with("$$") {
            if let Some(close_idx) = find_inline_math_end(s, i) {
                flush_plain(&mut plain_buf, plain_start, &mut separated, &mut atoms);
                atoms.push(InlineAtom::Raw {
                    text: s[i..=close_idx].to_string(),
                    separated,
                    start: i,
                });
                separated = Sep::None;
                i = close_idx + 1;
                continue;
            }
        }

        // Autolink / inline HTML tag / inline comment `<...>`
        if b == b'<' {
            if let Some(close_idx) = find_angle_span_end(s, i) {
                flush_plain(&mut plain_buf, plain_start, &mut separated, &mut atoms);
                atoms.push(InlineAtom::Raw {
                    text: s[i..=close_idx].to_string(),
                    separated,
                    start: i,
                });
                separated = Sep::None;
                i = close_idx + 1;
                continue;
            }
        }

        if allow_links {
            // Wikilink `[[...]]`
            if s[i..].starts_with("[[") {
                if let Some(close_start) = find_wikilink_end(s, i + 2) {
                    flush_plain(&mut plain_buf, plain_start, &mut separated, &mut atoms);
                    let inner = &s[i + 2..close_start];
                    if inner.contains(char::is_whitespace) {
                        atoms.push(InlineAtom::LinkGroup {
                            prefix: "[[".to_string(),
                            inner: inner.to_string(),
                            suffix: "]]".to_string(),
                            separated,
                            start: i,
                        });
                    } else {
                        atoms.push(InlineAtom::Raw {
                            text: s[i..close_start + 2].to_string(),
                            separated,
                            start: i,
                        });
                    }
                    separated = Sep::None;
                    i = close_start + 2;
                    continue;
                }
            }

            // Standard / Image / `@`-include / Space-recovered links: `![...]`, `@[...]`, `[...]`
            let link_prefix = if s[i..].starts_with("![") {
                Some("![")
            } else if s[i..].starts_with("@[") {
                Some("@[")
            } else if b == b'[' {
                Some("[")
            } else {
                None
            };

            if let Some(prefix) = link_prefix {
                let bracket_open = i + prefix.len() - 1;
                if let Some(bracket_close) = find_matching_bracket(s, bracket_open) {
                    let inner = &s[bracket_open + 1..bracket_close];
                    let after_bracket = bracket_close + 1;
                    if prefix == "![" {
                        let end = parse_link_suffix(s, after_bracket)
                            .map_or(after_bracket, |(end, _)| end);
                        flush_plain(&mut plain_buf, plain_start, &mut separated, &mut atoms);
                        atoms.push(InlineAtom::Raw {
                            text: s[i..end].to_string(),
                            separated,
                            start: i,
                        });
                        separated = Sep::None;
                        i = end;
                        continue;
                    }
                    if inner.starts_with('^') {
                        // Footnote labels cannot span lines, so keep the whole reference on one
                        // line, as written: breaking inside it would turn it into plain text, and
                        // joining a label that already spans lines would create a new
                        // definition/reference (or change an ordinary reference link's label).
                        flush_plain(&mut plain_buf, plain_start, &mut separated, &mut atoms);
                        atoms.push(InlineAtom::Raw {
                            text: s[i..=bracket_close].to_string(),
                            separated,
                            start: i,
                        });
                        separated = Sep::None;
                        i = after_bracket;
                        continue;
                    }
                    if let Some((suffix_end, suffix_str)) = parse_link_suffix(s, after_bracket) {
                        flush_plain(&mut plain_buf, plain_start, &mut separated, &mut atoms);
                        atoms.push(InlineAtom::LinkGroup {
                            prefix: prefix.to_string(),
                            inner: inner.to_string(),
                            suffix: suffix_str,
                            separated,
                            start: i,
                        });
                        separated = Sep::None;
                        i = suffix_end;
                        continue;
                    } else if prefix == "[" && is_plausible_shortcut_link(inner) {
                        flush_plain(&mut plain_buf, plain_start, &mut separated, &mut atoms);
                        atoms.push(InlineAtom::LinkGroup {
                            prefix: "[".to_string(),
                            inner: inner.to_string(),
                            suffix: "]".to_string(),
                            separated,
                            start: i,
                        });
                        separated = Sep::None;
                        i = after_bracket;
                        continue;
                    }
                }
            }
        }

        // Plain UTF-8 character
        if plain_buf.is_empty() {
            plain_start = i;
        }
        let ch = s[i..].chars().next().unwrap();
        plain_buf.push(ch);
        i += ch.len_utf8();
    }

    flush_plain(&mut plain_buf, plain_start, &mut separated, &mut atoms);
    atoms
}

fn atoms_to_tokens(atoms: Vec<InlineAtom>) -> Vec<WordToken> {
    // Expand each LinkGroup into concrete sub-chunks of (text, separated, penalty_before, is_link_start, link_group_idx)
    #[derive(Debug, Clone)]
    struct Piece {
        text: String,
        separated: Sep,
        penalty_before: i64,
        link_group_id: Option<usize>,
    }

    let mut pieces: Vec<Piece> = Vec::new();
    let mut next_group_id = 0usize;

    for atom in atoms {
        match atom {
            InlineAtom::Raw {
                text, separated, ..
            } => {
                pieces.push(Piece {
                    text,
                    separated,
                    penalty_before: 0,
                    link_group_id: None,
                });
            }
            InlineAtom::LinkGroup {
                prefix,
                inner,
                suffix,
                separated,
                ..
            } => {
                // Whitespace at the label's edges renders (`[ a ](u)` is `<a> a </a>`), so keep
                // one space there; inside the label it is ordinary, breakable whitespace.
                let mut prefix = prefix;
                let mut suffix = suffix;
                if inner.starts_with([' ', '\t', '\n', '\r']) {
                    prefix.push(' ');
                }
                if inner.ends_with([' ', '\t', '\n', '\r'])
                    && !inner.trim_matches([' ', '\t', '\n', '\r']).is_empty()
                {
                    suffix.insert(0, ' ');
                }
                let inner_atoms = scan_inline_atoms(&inner, false);
                let inner_tokens = atoms_to_tokens(inner_atoms);
                if inner_tokens.is_empty() {
                    pieces.push(Piece {
                        text: format!("{prefix}{suffix}"),
                        separated,
                        penalty_before: 0,
                        link_group_id: None,
                    });
                } else if inner_tokens.len() == 1 {
                    pieces.push(Piece {
                        text: format!("{prefix}{}{suffix}", inner_tokens[0].text),
                        separated,
                        penalty_before: 0,
                        link_group_id: None,
                    });
                } else {
                    let group_id = next_group_id;
                    next_group_id += 1;
                    let last_idx = inner_tokens.len() - 1;
                    for (idx, tok) in inner_tokens.into_iter().enumerate() {
                        let text = if idx == 0 {
                            format!("{prefix}{}", tok.text)
                        } else if idx == last_idx {
                            format!("{}{suffix}", tok.text)
                        } else {
                            tok.text
                        };
                        pieces.push(Piece {
                            text,
                            separated: if idx == 0 {
                                separated
                            } else if tok.newline_before {
                                Sep::Newline
                            } else {
                                Sep::Space
                            },
                            penalty_before: if idx == 0 { 0 } else { LINK_BREAK_PENALTY },
                            link_group_id: Some(group_id),
                        });
                    }
                }
            }
        }
    }

    // Merge unseparated pieces into atomic WordTokens.
    #[derive(Debug)]
    struct RawWord {
        text: String,
        penalty_before: i64,
        link_group_id: Option<usize>,
        newline_before: bool,
    }

    let mut merged: Vec<RawWord> = Vec::new();
    for piece in pieces {
        if piece.separated == Sep::None {
            if let Some(last) = merged.last_mut() {
                last.text.push_str(&piece.text);
                if last.link_group_id.is_none() {
                    last.link_group_id = piece.link_group_id;
                }
                continue;
            }
        }
        merged.push(RawWord {
            text: piece.text,
            penalty_before: piece.penalty_before,
            link_group_id: piece.link_group_id,
            newline_before: piece.separated == Sep::Newline,
        });
    }

    // Compute `link_group_width` for the start token of each link group after trailing punctuation gluing.
    let mut group_ranges: Vec<(usize, usize)> = Vec::new();
    let mut idx = 0;
    while idx < merged.len() {
        if let Some(gid) = merged[idx].link_group_id {
            let start = idx;
            while idx + 1 < merged.len() && merged[idx + 1].link_group_id == Some(gid) {
                idx += 1;
            }
            if idx > start {
                group_ranges.push((start, idx));
            }
        }
        idx += 1;
    }

    let mut group_widths: Vec<Option<usize>> = vec![None; merged.len()];
    for (start, end) in group_ranges {
        let mut total_width = 0usize;
        for (k, w) in merged[start..=end].iter().enumerate() {
            if k > 0 {
                total_width += 1;
            }
            total_width += UnicodeWidthStr::width(w.text.as_str());
        }
        group_widths[start] = Some(total_width);
    }

    merged
        .into_iter()
        .enumerate()
        .map(|(i, w)| WordToken::new(w.text, w.penalty_before, group_widths[i], w.newline_before))
        .collect()
}

/// Returns the end offset of the first backtick run of exactly `k` backticks at or after `start`.
pub(crate) fn find_closing_backticks(s: &str, start: usize, k: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = start;
    while i < len {
        if bytes[i] == b'`' {
            let mut run = 1;
            while i + run < len && bytes[i + run] == b'`' {
                run += 1;
            }
            if run == k {
                return Some(i + run);
            }
            i += run;
            continue;
        }
        i += 1;
    }
    None
}

fn find_inline_math_end(s: &str, open_idx: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    if open_idx + 1 >= len {
        return None;
    }
    // Opening `$` must not be followed by whitespace or `\n`
    if matches!(bytes[open_idx + 1], b' ' | b'\t' | b'\n' | b'\r') {
        return None;
    }

    let mut i = open_idx + 1;
    while i < len {
        let b = bytes[i];
        if b == b'\n' || b == b'\r' {
            return None;
        }
        if b == b'\\' && i + 1 < len {
            i += 2;
            continue;
        }
        if b == b'$' {
            // Do not match `$$`
            if i + 1 < len && bytes[i + 1] == b'$' {
                return None;
            }
            // Closing `$` must not be preceded by whitespace
            if matches!(bytes[i - 1], b' ' | b'\t') {
                return None;
            }
            // Closing `$` must not be followed by ASCII alphanumeric (avoids currency like `$100-$200`)
            if i + 1 < len && bytes[i + 1].is_ascii_alphanumeric() {
                return None;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_angle_span_end(s: &str, open_idx: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    if open_idx + 1 >= len {
        return None;
    }

    // HTML comment `<!-- ... -->` on same line
    if s[open_idx..].starts_with("<!--") {
        let line_end = s[open_idx..]
            .find('\n')
            .map(|p| open_idx + p)
            .unwrap_or(len);
        if let Some(rel) = s[open_idx + 4..line_end].find("-->") {
            return Some(open_idx + 4 + rel + 2);
        }
        return None;
    }

    let next = bytes[open_idx + 1];
    // Must look like an HTML tag (`<tag`, `</tag`, `<!...`, `<?...`) or autolink (`<scheme:...>` / `<a@b>`)
    let starts_as_tag = next == b'/' || next == b'!' || next == b'?' || next.is_ascii_alphabetic();
    if !starts_as_tag {
        return None;
    }

    let mut i = open_idx + 1;
    let mut quote: Option<u8> = None;
    while i < len {
        let b = bytes[i];
        if b == b'\n' || b == b'\r' {
            return None;
        }
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else {
            if b == b'"' || b == b'\'' {
                quote = Some(b);
            } else if b == b'<' {
                return None;
            } else if b == b'>' {
                let inner = &s[open_idx + 1..i];
                if inner.is_empty() {
                    return None;
                }
                if inner.contains(' ') {
                    // Must look like an HTML tag with attributes, not comparison `a < b > c`
                    let valid_tag = inner.starts_with('/')
                        || inner.bytes().take_while(|&c| c != b' ').all(|c| {
                            c.is_ascii_alphanumeric() || c == b'-' || c == b':' || c == b'_'
                        });
                    if !valid_tag {
                        return None;
                    }
                    return Some(i);
                } else if inner.contains("://")
                    || inner.contains('@')
                    || inner.starts_with("mailto:")
                    || inner.starts_with('/')
                    || inner
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'/' || c == b'_')
                {
                    return Some(i);
                } else {
                    return None;
                }
            }
        }
        i += 1;
    }
    None
}

fn find_wikilink_end(s: &str, start: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = start;
    while i + 1 < len {
        if bytes[i] == b'\n' || bytes[i] == b'\r' {
            return None;
        }
        if bytes[i] == b'\\' && i + 2 < len {
            i += 2;
            continue;
        }
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            return None;
        }
        if bytes[i] == b']' && bytes[i + 1] == b']' {
            if i == start {
                return None;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_matching_bracket(s: &str, open_idx: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut depth = 1usize;
    let mut i = open_idx + 1;

    while i < len {
        let b = bytes[i];
        if b == b'\\' && i + 1 < len {
            i += 2;
            continue;
        }
        if b == b'`' {
            let mut k = 1;
            while i + k < len && bytes[i + k] == b'`' {
                k += 1;
            }
            if let Some(end) = find_closing_backticks(s, i + k, k) {
                i = end;
                continue;
            }
            i += k;
            continue;
        }
        if b == b'[' {
            depth += 1;
        } else if b == b']' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }

    None
}

fn parse_link_suffix(s: &str, start: usize) -> Option<(usize, String)> {
    let bytes = s.as_bytes();
    if start >= bytes.len() {
        return None;
    }

    // Inline link destination `(dest)`
    if bytes[start] == b'(' {
        if let Some((close_idx, raw_inner)) = scan_parenthesized_destination(s, start) {
            if is_valid_or_recovered_destination(raw_inner) {
                // Keep exact source slice `(dest)` verbatim!
                let suffix = format!("]{}", &s[start..=close_idx]);
                return Some((close_idx + 1, suffix));
            }
        }
        return None;
    }

    // Reference link `[ref]` or `[]`
    if bytes[start] == b'[' {
        let mut i = start + 1;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if bytes[i] == b'[' {
                return None;
            }
            if bytes[i] == b']' {
                let suffix = format!("]{}", &s[start..=i]);
                return Some((i + 1, suffix));
            }
            i += 1;
        }
    }

    None
}

fn scan_parenthesized_destination(s: &str, open_paren: usize) -> Option<(usize, &str)> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = open_paren + 1;
    let mut paren_depth = 1usize;
    let mut in_quotes: Option<u8> = None;
    let mut in_angle = false;

    while i < len {
        let b = bytes[i];
        if b == b'\\' && i + 1 < len {
            i += 2;
            continue;
        }
        if let Some(q) = in_quotes {
            if b == q {
                in_quotes = None;
            }
        } else if in_angle {
            if b == b'>' {
                in_angle = false;
            }
        } else {
            if b == b'<' && i == open_paren + 1 {
                in_angle = true;
            } else if b == b'"' || b == b'\'' {
                in_quotes = Some(b);
            } else if b == b'(' {
                paren_depth += 1;
            } else if b == b')' {
                paren_depth -= 1;
                if paren_depth == 0 {
                    return Some((i, &s[open_paren + 1..i]));
                }
            }
        }
        i += 1;
    }
    None
}

fn is_valid_or_recovered_destination(raw_inner: &str) -> bool {
    let trimmed = raw_inner.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.starts_with('<') {
        return trimmed.contains('>');
    }

    // Separate optional trailing title `"..."` or `'...'`
    let dest = strip_optional_trailing_title(trimmed);
    if !dest.contains([' ', '\t']) {
        return true;
    }

    // Space-containing destination recovery check:
    let lower = dest.to_ascii_lowercase();
    if lower.starts_with("file://")
        || lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("./")
        || lower.starts_with("../")
        || lower.starts_with("~/")
        || lower.starts_with('/')
        || lower.starts_with('#')
        || dest.contains('/')
        || dest.contains('\\')
    {
        return true;
    }

    // Ends with `.<ext>` (1..=8 ASCII alphanumeric chars), e.g., `My Document.pdf`
    if let Some(dot_pos) = dest.rfind('.') {
        let ext = &dest[dot_pos + 1..];
        if (1..=8).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return true;
        }
    }

    false
}

fn strip_optional_trailing_title(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && (bytes[bytes.len() - 1] == b'"' || bytes[bytes.len() - 1] == b'\'') {
        let q = bytes[bytes.len() - 1];
        // Find matching opening quote preceded by whitespace
        let mut i = bytes.len() - 2;
        while i > 0 {
            if bytes[i] == q && bytes[i - 1] != b'\\' && bytes[i - 1].is_ascii_whitespace() {
                return s[..i].trim_end();
            }
            i -= 1;
        }
    }
    s
}

fn is_plausible_shortcut_link(inner: &str) -> bool {
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Exclude GFM checkboxes `[ ]`, `[x]`, `[X]`, GitHub alerts `[!NOTE]`, footnotes `[^id]`
    if matches!(trimmed, "x" | "X") || trimmed.starts_with('!') || trimmed.starts_with('^') {
        return false;
    }
    true
}
