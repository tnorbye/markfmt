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

//! Line endings, the final newline, and byte order marks: formatting a file saved with CRLF, a
//! BOM, or no final newline must give exactly the LF result with those properties re-applied.

use std::fs;
use std::path::Path;
use std::process::Command;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use markfmt::formatter::format_file_content;
use markfmt::range_filter::RangeFilter;
use markfmt::{format_markdown, FormattingOptions};

/// LF documents ending in a newline, covering the block types whose raw lines are kept.
const CASES: &[&str] = &[
    "- [ ] \n  continued\n",
    "- [x] \n\n  Second.\n",
    ">  [!TIP]  \n> body\n",
    "R[^1].\n\n[^1]: a  \n===\n",
    "Foo.[^one two]\n\n[^one two]: Here is the footnote.\n",
    "![ a  b ](/img)\n",
    "Plain   paragraph that is long enough to wrap around at the configured width here.\n",
    "---\ntitle: front   matter\n---\n\n# Heading   text\n\nBody   text.\n",
    "- item one with words that wrap around\n- item two\n\n      indented code\n",
    "1. first\n\n   ```rust\n   fn  main() {}\n   ```\n\n2. second   item\n",
    "> quoted   text that wraps around the margin\n> more quoted\n",
    "| a | b |\n|---|:-:|\n| 1 | 2 |\n",
    "Hard   break  \nnext line\\\nlast line\n",
    "<!-- markfmt:off -->\nkeep   this\n<!-- markfmt:on -->\n\ntext   after\n",
    "<details>\n<summary>Sum</summary>\n\nInner   text.\n\n</details>\n",
    "$$\nx  =  1\n$$\n\nText.\n",
    "* Compile docs\n\n Publish the pages.\n",
    "- ```\n  <rules>\n  a   b\n  </rules>\n  ```\n",
    "Text `one  \n two` and [a](url\n \"title\").\n",
    "#hashtag\n===\n",
    "one  \ntwo\n===\n",
    "<!-- prettier-ignore -->\n- keep   first\n\n- keep   second\n",
    "<pre>\n<rules>\na   b\n</rules>\n</pre>\n",
    "```\ncode\n\n\n",
    "**Title**\n    Author\n\nTerm\n:   Long definition body that can wrap.\n\n<script src=\"markdeep.min.js\"></script>\n",
    "Text with trailing blank lines.\n\n\n",
];

fn opts() -> FormattingOptions {
    let mut opts = FormattingOptions::default();
    opts.max_line_width = 30;
    opts
}

fn crlf(s: &str) -> String {
    s.replace('\n', "\r\n")
}

fn without_final_newline(s: &str) -> &str {
    s.strip_suffix("\r\n")
        .or_else(|| s.strip_suffix('\n'))
        .unwrap_or(s)
}

#[test]
fn crlf_files_format_like_lf_files() {
    for case in CASES {
        let expected = crlf(&format_markdown(case, &opts()));
        let actual = format_markdown(&crlf(case), &opts());
        assert_eq!(actual, expected, "input: {case:?}");
        assert!(
            !actual.replace("\r\n", "").contains('\n'),
            "bare LF in {actual:?}"
        );
        assert_eq!(format_markdown(&actual, &opts()), actual, "not idempotent");
    }
}

#[test]
fn missing_final_newline_is_preserved() {
    for case in CASES {
        for input in [case.to_string(), crlf(case)] {
            let input_without = without_final_newline(without_final_newline(&input).trim_end());
            // A one-line file has no line endings left to detect, so new ones default to LF.
            let source = if input_without.contains('\n') {
                format!(
                    "{input_without}{}",
                    if input.contains("\r\n") { "\r\n" } else { "\n" }
                )
            } else {
                format!("{input_without}\n")
            };
            let expected = format_markdown(&source, &opts());
            let actual = format_markdown(input_without, &opts());
            assert_eq!(
                actual,
                without_final_newline(&expected),
                "input: {input_without:?}"
            );
            assert_eq!(format_markdown(&actual, &opts()), actual, "not idempotent");
        }
    }
}

#[test]
fn byte_order_mark_is_preserved_and_ignored_for_formatting() {
    for case in CASES {
        for input in [case.to_string(), crlf(case)] {
            let expected = format!("\u{feff}{}", format_markdown(&input, &opts()));
            assert_eq!(
                format_markdown(&format!("\u{feff}{input}"), &opts()),
                expected,
                "input: {input:?}"
            );
        }
    }
    // Front matter after a BOM is still recognized (and so left alone).
    let doc = "\u{feff}---\ntitle: x\n---\nText   here\n";
    assert_eq!(
        format_markdown(doc, &opts()),
        "\u{feff}---\ntitle: x\n---\n\nText here\n"
    );
}

#[test]
fn whitespace_only_files_keep_their_final_newline_state() {
    assert_eq!(format_markdown("", &opts()), "");
    assert_eq!(format_markdown("   ", &opts()), "");
    assert_eq!(format_markdown("  \n\n", &opts()), "\n");
    assert_eq!(format_markdown("  \r\n\r\n", &opts()), "\r\n");
}

#[test]
fn mixed_line_endings_follow_the_predominant_style() {
    // Two CRLF lines and one LF line: CRLF wins even though the first line is LF.
    assert_eq!(
        format_markdown("a\nb\r\n\r\nc\r\n", &opts()),
        "a b\r\n\r\nc\r\n"
    );
    assert_eq!(format_markdown("a\r\nb\n\nc\n", &opts()), "a b\n\nc\n");
}

#[test]
fn partial_formatting_keeps_line_endings_and_end_of_file() {
    let path = Path::new("/virtual/doc.md");
    let only_line_3 = RangeFilter::from_line_strings(path, &["3".to_string()]).unwrap();
    for (input, expected) in [
        (
            "Para one   here.\r\n\r\nPara two   here.",
            "Para one   here.\r\n\r\nPara two here.",
        ),
        (
            "Para one   here.\n\nPara two   here.\n",
            "Para one   here.\n\nPara two here.\n",
        ),
    ] {
        assert_eq!(
            format_file_content(input, path, &only_line_3, &opts()),
            expected
        );
    }
}

#[test]
fn editorconfig_insert_final_newline_overrides_preservation() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let doc = root.join("doc.md");
    let run = |config: &str, content: &str| {
        fs::write(root.join(".editorconfig"), config).unwrap();
        fs::write(&doc, content).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_markfmt"))
            .arg(&doc)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        fs::read_to_string(&doc).unwrap()
    };

    let add = "root = true\n[*.md]\ninsert_final_newline = true\n";
    assert_eq!(run(add, "Some   text"), "Some text\n");
    assert_eq!(run(add, "Some   text\r\n"), "Some text\r\n");
    let remove = "root = true\n[*.md]\ninsert_final_newline = false\n";
    assert_eq!(run(remove, "Some   text\n"), "Some text");
    let preserve = "root = true\n[*.md]\n";
    assert_eq!(run(preserve, "Some   text"), "Some text");
    assert_eq!(run(preserve, "Some   text\n"), "Some text\n");
}
