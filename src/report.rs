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

//! Terminal output: styled messages, run summaries, and diffs.
//!
//! Messages go through `anstream`, which drops ANSI styling unless the stream is a terminal
//! that supports color (and honors `NO_COLOR`, `CLICOLOR`, `CLICOLOR_FORCE`, and `--color`).
//! Formatted document content never goes through it, so it is always written byte-for-byte.

use std::io::{self, Write};

use anstream::ColorChoice;
use anstyle::{AnsiColor, Effects, Style};
use similar::{capture_diff_slices, Algorithm, DiffOp, DiffTag, TextDiff};

use markfmt::cli::ColorWhen;

const ERROR: Style = AnsiColor::Red.on_default().effects(Effects::BOLD);
const WARNING: Style = AnsiColor::Yellow.on_default().effects(Effects::BOLD);
const PATH: Style = Style::new().effects(Effects::BOLD);
const DIFF_HEADER: Style = Style::new().effects(Effects::BOLD);
const HUNK_HEADER: Style = AnsiColor::Cyan.on_default();
const DELETED: Style = AnsiColor::Red.on_default();
const INSERTED: Style = AnsiColor::Green.on_default();
/// Words that actually changed within a changed line.
const EMPHASIS: Effects = Effects::INVERT;

/// Applies `--color` to all subsequent output.
pub fn apply_color(when: ColorWhen) {
    let choice = match when {
        ColorWhen::Auto => ColorChoice::Auto,
        ColorWhen::Always => ColorChoice::Always,
        ColorWhen::Never => ColorChoice::Never,
    };
    choice.write_global();
}

pub fn error(message: &str) {
    anstream::eprintln!("{ERROR}Error:{ERROR:#} {message}");
}

pub fn warning(message: &str) {
    anstream::eprintln!("{WARNING}Warning:{WARNING:#} {message}");
}

/// `Would reformat <path>` on stderr, for `--check`.
pub fn would_reformat(name: &str) {
    anstream::eprintln!("Would reformat {PATH}{name}{PATH:#}");
}

/// `Formatted <path>` on stdout, for `--verbose`.
pub fn formatted(name: &str) {
    anstream::println!("Formatted {PATH}{name}{PATH:#}");
}

/// Per-run file counts for the closing summary line.
#[derive(Debug, Default)]
pub struct Counts {
    pub changed: usize,
    pub unchanged: usize,
    pub failed: usize,
}

fn files(n: usize) -> String {
    if n == 1 {
        "1 file".to_string()
    } else {
        format!("{n} files")
    }
}

impl Counts {
    /// Summary after formatting, for example `2 files reformatted, 46 files unchanged`.
    pub fn print_formatted_summary(&self) {
        let mut parts = Vec::new();
        if self.changed > 0 {
            parts.push(format!("{} reformatted", files(self.changed)));
        }
        if self.unchanged > 0 {
            parts.push(format!("{} unchanged", files(self.unchanged)));
        }
        self.push_failed(&mut parts);
        if parts.is_empty() {
            anstream::println!("No files to format");
        } else {
            anstream::println!("{}", parts.join(", "));
        }
    }

    /// Summary after `--check` (on stderr, like the `Would reformat` lines), with a hint
    /// when files need formatting.
    pub fn print_check_summary(&self) {
        let mut parts = Vec::new();
        if self.changed > 0 {
            parts.push(format!(
                "{WARNING}{} would be reformatted{WARNING:#}",
                files(self.changed)
            ));
        }
        if self.unchanged > 0 {
            parts.push(format!("{} already formatted", files(self.unchanged)));
        }
        self.push_failed(&mut parts);
        if parts.is_empty() {
            anstream::eprintln!("No files to check");
            return;
        }
        anstream::eprintln!("{}", parts.join(", "));
        if self.changed > 0 {
            anstream::eprintln!("Run markfmt without --check to apply the changes.");
        }
    }

    fn push_failed(&self, parts: &mut Vec<String>) {
        if self.failed > 0 {
            parts.push(format!(
                "{ERROR}{} could not be processed{ERROR:#}",
                files(self.failed)
            ));
        }
    }
}

/// Prints a unified diff from `original` to `formatted` on stdout.
///
/// When stdout is not a color terminal this is a standard unified diff (usable with `patch`
/// and `git apply`). On a color terminal, removed and added lines are red and green, and words
/// whose text changed are highlighted. Words are compared regardless of where lines break, so a
/// paragraph that was only rewrapped shows no highlighting at all.
pub fn print_diff(name: &str, original: &str, formatted: &str) -> io::Result<()> {
    let diff = TextDiff::from_lines(original, formatted);
    let old_header = format!("a/{name}");
    let new_header = format!("b/{name}");
    let mut udiff = diff.unified_diff();
    udiff.context_radius(3).header(&old_header, &new_header);

    let mut out = io::stdout().lock();
    if anstream::AutoStream::choice(&io::stdout()) == ColorChoice::Never {
        return write!(out, "{udiff}");
    }

    writeln!(out, "{DIFF_HEADER}--- {old_header}{DIFF_HEADER:#}")?;
    writeln!(out, "{DIFF_HEADER}+++ {new_header}{DIFF_HEADER:#}")?;
    let old_lines = diff.old_slices();
    let new_lines = diff.new_slices();
    for hunk in udiff.iter_hunks() {
        writeln!(out, "{HUNK_HEADER}{}{HUNK_HEADER:#}", hunk.header())?;
        for op in hunk.ops() {
            let (tag, old_range, new_range) = op.as_tag_tuple();
            let old = &old_lines[old_range];
            let new = &new_lines[new_range];
            match tag {
                DiffTag::Equal => write_lines(&mut out, ' ', Style::new(), old, None)?,
                DiffTag::Delete => write_lines(&mut out, '-', DELETED, old, None)?,
                DiffTag::Insert => write_lines(&mut out, '+', INSERTED, new, None)?,
                DiffTag::Replace => {
                    let (old_changed, new_changed) = changed_words(old, new);
                    write_lines(&mut out, '-', DELETED, old, Some(&old_changed))?;
                    write_lines(&mut out, '+', INSERTED, new, Some(&new_changed))?;
                }
            }
        }
    }
    Ok(())
}

/// Splits `line` into alternating whitespace and word runs.
fn runs(line: &str) -> impl Iterator<Item = (bool, &str)> {
    let mut rest = line;
    std::iter::from_fn(move || {
        let first = rest.chars().next()?;
        let is_space = first.is_whitespace();
        let end = rest
            .find(|c: char| c.is_whitespace() != is_space)
            .unwrap_or(rest.len());
        let (run, tail) = rest.split_at(end);
        rest = tail;
        Some((!is_space, run))
    })
}

fn words<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    lines
        .iter()
        .flat_map(|line| runs(line).filter(|(is_word, _)| *is_word).map(|(_, w)| w))
        .collect()
}

/// For each word (in order) of `old` and of `new`, whether it differs from the other side.
fn changed_words(old: &[&str], new: &[&str]) -> (Vec<bool>, Vec<bool>) {
    let old_words = words(old);
    let new_words = words(new);
    let mut old_changed = vec![true; old_words.len()];
    let mut new_changed = vec![true; new_words.len()];
    for op in capture_diff_slices(Algorithm::Patience, &old_words, &new_words) {
        if let DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            old_changed[old_index..old_index + len].fill(false);
            new_changed[new_index..new_index + len].fill(false);
        }
    }
    (old_changed, new_changed)
}

/// Writes diff lines with a `sign` prefix in `style`, highlighting changed words when
/// `changed` (one flag per word across all `lines`) is given.
fn write_lines(
    out: &mut impl Write,
    sign: char,
    style: Style,
    lines: &[&str],
    changed: Option<&[bool]>,
) -> io::Result<()> {
    let emphasis = style.effects(EMPHASIS);
    let mut word_index = 0;
    for line in lines {
        let content = line.trim_end_matches(['\n', '\r']);
        write!(out, "{style}{sign}")?;
        for (is_word, run) in runs(content) {
            let highlight = is_word && changed.is_some_and(|c| c[word_index]);
            if is_word {
                word_index += 1;
            }
            if highlight {
                write!(out, "{emphasis}{run}{emphasis:#}{style}")?;
            } else {
                write!(out, "{run}")?;
            }
        }
        writeln!(out, "{style:#}")?;
        if !line.ends_with('\n') {
            writeln!(out, "\\ No newline at end of file")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::changed_words;

    #[test]
    fn rewrapping_changes_no_words_but_edits_do() {
        let old = ["one two   three four\n"];
        let new = ["one two\n", "three four\n"];
        let (old_changed, new_changed) = changed_words(&old, &new);
        assert!(old_changed.iter().chain(&new_changed).all(|c| !c));

        let (old_changed, new_changed) = changed_words(&["|-|\n"], &["| --- |\n"]);
        assert_eq!(old_changed, [true]);
        assert_eq!(new_changed, [true, true, true]);
    }
}
