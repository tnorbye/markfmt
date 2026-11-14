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

use crate::inline::{WordToken, LINK_BREAK_PENALTY};
use crate::options::WrapMode;

const LINE_PENALTY: i64 = 50;
const RUNT_PENALTY: i64 = 100;
/// Greedy-mode counterpart of `LINK_BREAK_PENALTY`: move a whole multi-word link to the next
/// line only if that leaves at most this many columns of slack (`slack^2 < LINK_BREAK_PENALTY`).
const LINK_MAX_SLACK: usize = 34;
const _: () = assert!(
    (LINK_MAX_SLACK * LINK_MAX_SLACK) as i64 <= LINK_BREAK_PENALTY
        && ((LINK_MAX_SLACK + 1) * (LINK_MAX_SLACK + 1)) as i64 > LINK_BREAK_PENALTY
);
const OVERFLOW_BASE_PENALTY: i64 = 50_000_000;
const INF: i64 = i64::MAX / 4;

/// Breaks one hard-break-free run of paragraph text into lines according to `mode`.
pub fn reflow_segment(
    tokens: &[WordToken],
    first_width: usize,
    rest_width: usize,
    mode: WrapMode,
    optimal: bool,
) -> Vec<String> {
    match mode {
        WrapMode::Always => reflow_tokens(tokens, first_width, rest_width, optimal),
        WrapMode::Never => join_lines(tokens, |_| false),
        WrapMode::Preserve => join_lines(tokens, |i| tokens[i].newline_before),
        WrapMode::Sentence => {
            let mut lines = Vec::new();
            let mut start = 0;
            for end in 1..=tokens.len() {
                if end == tokens.len() || ends_sentence(&tokens[end - 1].text, &tokens[end].text) {
                    let width = if start == 0 { first_width } else { rest_width };
                    lines.extend(reflow_tokens(
                        &tokens[start..end],
                        width,
                        rest_width,
                        optimal,
                    ));
                    start = end;
                }
            }
            lines
        }
    }
}

/// Joins tokens with single spaces, starting a new line before token `i` when `breaks_before(i)`.
fn join_lines(tokens: &[WordToken], breaks_before: impl Fn(usize) -> bool) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (i, tok) in tokens.iter().enumerate() {
        match lines.last_mut() {
            Some(line) if !breaks_before(i) => {
                line.push(' ');
                line.push_str(&tok.text);
            }
            _ => lines.push(tok.text.clone()),
        }
    }
    lines
}

/// Common abbreviations that end with a period but rarely end a sentence.
const ABBREVIATIONS: &[&str] = &[
    "e.g.", "i.e.", "etc.", "vs.", "cf.", "al.", "approx.", "fig.", "no.", "mr.", "mrs.", "ms.",
    "dr.", "prof.", "st.", "jr.", "sr.", "inc.", "ltd.", "co.",
];

/// Heuristic sentence boundary between adjacent words: `word` ends with `.`, `!`, or `?`
/// (optionally followed by closing quotes, brackets, or emphasis), is not a common abbreviation
/// or an initial, and `next` starts like a new sentence.
fn ends_sentence(word: &str, next: &str) -> bool {
    let core = word.trim_end_matches([')', '"', '\'', '\u{201d}', '\u{2019}', '*', '_']);
    if !core.ends_with(['.', '!', '?']) {
        return false;
    }
    let bare = core.trim_start_matches(['(', '"', '\'', '\u{201c}', '\u{2018}', '*', '_']);
    let lower = bare.to_lowercase();
    if ABBREVIATIONS.contains(&lower.as_str()) {
        return false;
    }
    // Initials such as `J.` and list-like numbers such as `3.`.
    let stem = &bare[..bare.len() - 1];
    if stem.chars().count() == 1 || (!stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }

    let next = next.trim_start_matches(['(', '"', '\'', '\u{201c}', '\u{2018}', '*', '_']);
    let mut chars = next.chars();
    match chars.next() {
        Some(c) if c.is_uppercase() || c.is_ascii_digit() || c == '`' => true,
        Some('[') => chars.next().is_some_and(char::is_uppercase),
        _ => false,
    }
}

/// Reflows tokenized words into lines (never breaking inside a token; tokens that must not start
/// or end a line are already glued together by the tokenizer) using greedy wrapping for `<= 3` lines (or when
/// `optimal == false`) and Knuth-Plass squared-slack DP with zero last-line slack cost for `4+` lines.
fn reflow_tokens(
    tokens: &[WordToken],
    first_width: usize,
    rest_width: usize,
    optimal: bool,
) -> Vec<String> {
    if tokens.is_empty() {
        return Vec::new();
    }
    if tokens.len() == 1 {
        return vec![tokens[0].text.clone()];
    }

    let first_limit = first_width.max(1);
    let rest_limit = rest_width.max(1);

    let greedy = reflow_greedy(tokens, first_limit, rest_limit);
    if !optimal || greedy.len() <= 3 {
        return greedy;
    }

    reflow_optimal(tokens, first_limit, rest_limit)
}

fn reflow_greedy(tokens: &[WordToken], first_width: usize, rest_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut col = 0usize;
    let mut limit = first_width;

    for tok in tokens {
        if col == 0 {
            current.push_str(&tok.text);
            col = tok.width;
            continue;
        }

        // Smart link lookahead: avoid breaking `[multi word](url)` across lines when the whole link
        // fits on the next line and the current line wouldn't leave an excessively huge gap.
        if let Some(gw) = tok.link_group_width {
            if col + 1 + gw > limit
                && gw <= rest_width
                && limit.saturating_sub(col) <= LINK_MAX_SLACK
            {
                lines.push(std::mem::take(&mut current));
                limit = rest_width;
                current.push_str(&tok.text);
                col = tok.width;
                continue;
            }
        }

        if col + 1 + tok.width <= limit {
            current.push(' ');
            current.push_str(&tok.text);
            col += 1 + tok.width;
        } else {
            lines.push(std::mem::take(&mut current));
            limit = rest_width;
            current.push_str(&tok.text);
            col = tok.width;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}

fn reflow_optimal(tokens: &[WordToken], first_width: usize, rest_width: usize) -> Vec<String> {
    let n = tokens.len();
    let mut min_cost = vec![INF; n + 1];
    let mut prev_break = vec![0usize; n + 1];
    min_cost[0] = 0;

    for i in 0..n {
        if min_cost[i] >= INF {
            continue;
        }
        let limit = if i == 0 { first_width } else { rest_width };
        // Tokens carry `penalty_before == LINK_BREAK_PENALTY` on internal link boundaries.
        let break_pen = if i > 0 { tokens[i].penalty_before } else { 0 };

        let mut width = 0usize;
        for j in (i + 1)..=n {
            if j > i + 1 {
                width += 1;
            }
            width += tokens[j - 1].width;

            if width > limit {
                if j == i + 1 {
                    let overflow = (width - limit).min(10_000) as i64;
                    let cost =
                        min_cost[i] + break_pen + OVERFLOW_BASE_PENALTY + overflow * overflow;
                    if cost < min_cost[j] {
                        min_cost[j] = cost;
                        prev_break[j] = i;
                    }
                }
                break;
            }

            let slack_cost = if j == n {
                if j == i + 1 && tokens[i].width <= 4 {
                    RUNT_PENALTY
                } else {
                    0
                }
            } else {
                let slack = (limit - width).min(10_000) as i64;
                slack * slack
            };

            let total = min_cost[i] + break_pen + LINE_PENALTY + slack_cost;
            if total < min_cost[j] {
                min_cost[j] = total;
                prev_break[j] = i;
            }
        }
    }

    let mut reversed_lines = Vec::new();
    let mut j = n;
    while j > 0 {
        let i = prev_break[j];
        let line = tokens[i..j]
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        reversed_lines.push(line);
        j = i;
    }
    reversed_lines.reverse();
    reversed_lines
}

#[cfg(test)]
mod tests {
    use super::ends_sentence;

    #[test]
    fn detects_sentence_boundaries() {
        assert!(ends_sentence("done.", "Next"));
        assert!(ends_sentence("really?", "Yes"));
        assert!(ends_sentence("(done.)", "Then"));
        assert!(ends_sentence("**done.**", "`code`"));
        assert!(!ends_sentence("e.g.", "Rust"));
        assert!(!ends_sentence("J.", "Smith"));
        assert!(!ends_sentence("step", "Two"));
        assert!(!ends_sentence("done.", "then"));
        assert!(!ends_sentence("3.", "Later"));
    }
}
