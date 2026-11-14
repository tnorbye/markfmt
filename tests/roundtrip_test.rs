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

//! Round-trip invariants: formatting must be idempotent and must not change how the document
//! renders. Rendering equivalence is checked by comparing pulldown-cmark HTML output (with
//! only ordinary text whitespace normalized; code, math, and HTML attributes stay exact).

use markfmt::{FormattingOptions, OrderedListStyle, WrapMode};
mod common;
use common::check_round_trip;

fn check(input: &str, base: &FormattingOptions, width: usize) -> Option<String> {
    let mut opts = base.clone();
    opts.max_line_width = width;
    check_round_trip(input, &opts)
}

fn assert_all(cases: &[String], widths: impl Iterator<Item = usize> + Clone) {
    assert_all_with(cases, &FormattingOptions::default(), widths);
}

fn assert_all_with(
    cases: &[String],
    base: &FormattingOptions,
    widths: impl Iterator<Item = usize> + Clone,
) {
    let mut failures = Vec::new();
    for case in cases {
        for w in widths.clone() {
            if let Some(f) = check(case, base, w) {
                failures.push(f);
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases failed; first failures:\n\n{}",
        failures.len(),
        cases.len(),
        failures
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n==========\n\n")
    );
}

/// Tiny deterministic PRNG so the test needs no extra dependencies.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as usize
    }
}

/// Words that are harmless mid-line but change block structure (or the hard-break status of the
/// line) if a reflow places them at the start or end of a line.
const HAZARDS: &[&str] = &[
    "1.",
    "1)",
    "-",
    "+",
    "*",
    "#",
    "##",
    "=",
    "==",
    "--",
    "---",
    "***",
    "___",
    "_",
    "[foo]:",
    ">",
    ">x",
    "```",
    "~~~",
    "<div>",
    "</div>",
    "<p>",
    "<pre>",
    "<!--x-->",
    "<?x?>",
    "<!DOCTYPE>",
    "<example>",
    "[^1]:",
    "$$x$$",
    "@docs/x.md",
    "foo\\",
    "|",
    "|-|",
    ":-:",
    "!!!",
    "!!!:Note",
    ":",
    "*+**",
    "<!--@x-->",
    "<a href=\"https://e.x\">",
    "<Tag>x</Tag>",
    "<urn:isbn:1>",
];

const WORDS: &[&str] = &[
    "alpha",
    "be",
    "c",
    "delta",
    "echo",
    "fox",
    "golf",
    "hotel",
    "in",
    "a",
    "`code span`",
    "[link text](http://e.x)",
    "<https://e.x>",
    "**bold**",
    "_em_",
    "x",
];

fn random_paragraph(rng: &mut Lcg) -> String {
    let len = 4 + rng.next() % 30;
    let mut words = vec!["Start".to_string()];
    for _ in 0..len {
        let w = if rng.next() % 3 == 2 {
            HAZARDS[rng.next() % HAZARDS.len()]
        } else {
            WORDS[rng.next() % WORDS.len()]
        };
        words.push(w.to_string());
    }
    // Never end on a backslash: a trailing `\` at the end of a paragraph is literal text, which
    // makes the generated input itself ambiguous to compare.
    words.push("end.".to_string());
    words.join(" ")
}

#[test]
fn random_paragraphs_with_hazard_words_round_trip() {
    let mut rng = Lcg(0x5eed);
    let mut cases = Vec::new();
    for _ in 0..250 {
        let p = random_paragraph(&mut rng);
        cases.push(format!("{p}\n"));
        cases.push(format!("- {p}\n- second item\n"));
        cases.push(format!("> {p}\n"));
        cases.push(format!("1. {p}\n\n   {p}\n"));
    }
    assert_all(&cases, (6..=40).step_by(2));
}

/// Minimal reproductions of hazards that previously changed block structure.
#[test]
fn line_start_and_end_hazards_round_trip() {
    let cases: Vec<String> = [
        "--- aaaaaaaaa bbbbbbbbb\n",
        "*** aaaaaaaaa bbbbbbbbb\n",
        "<custom> aaaaaaaaa bbbbbbbbb\n",
        "[release]:\n/releases aaaaaaaaaa bbbbbbbbbb\n",
        "_ _ _ _ placeholder\n",
        "[release]: /releases \"Release notes\" draft\n",
        "| Command | Purpose |\n| --- |\n| check |\n",
        "[guide](<docs/first\nsecond.md>)\n",
        "[^build\ncache]: documentation text\n",
        "xxxxxxxxxx 1. yyy\n",                                 // ordered list
        "xxxxxxxxxx >yy\n",                                    // blockquote
        "xxxxxxxxxx =\n",                                      // setext heading
        "Some words here and then <div> appears in text ok\n", // HTML block
        "aaaa bbbb cccc dddd <!-- note --> end\n",             // HTML comment block
        "aaaaaaaaaaaa [^1]: x\n",                              // footnote definition
        "Start `code span` |-| more words\n",                  // table delimiter row
        "words before C:\\ and after\n",                       // trailing backslash => hard break
        "math in the middle\n$$x$$\nof a paragraph\n",         // inline display math line
        "see[ link ](https://x.y)here and [ two words ](u) too\n", // spaces at label edges
        "a [ b][r] c ![ alt ](i.png) d [\nlabel](u)\n\n[r]: /u\n", // reference, image, newline
        "words then a reference [^a b c] at the end of it\n\n[^a b c]: Note.\n", // footnote ref
        ">\n> [!NOTE]\n",                                      // blank quote before alert text
        ">\n> [!NOTE]  \n> body\n",                            // blank quote + hard break
        "[id]: https://e.x\nOrdinary words after a definition wrap.\n", // prose after definition
        "[Note]: This is a note with enough words to wrap.\n", // rejected definition prefix
        "[release]: /releases\n\"Release notes\" draft with more words.\n", // title hazard prefix
        "Intro words <div data-url=\"https://e.x\"> *em* and more words\n", // `://` in a tag
        "Intro words <!-- @todo --> *em* and more words\n",   // `@` in a comment
        "<custom url=\"https://e.x\"> *em* and more words\n", // tag alone on line 0
        "<custom email=\"a@b.c\"> *em* and more words\n",
        "Words <https://e.x/a> and <a@b.co> are autolinks here\n", // autolinks may start lines
        "$$a$ $b$$ more words\n",                               // display math line 0
        "<Tag>inline text</Tag> 12345678 12345678 12345 12345678 12345678 12345 ab\n", // prompt tag
        "*+** *em* *+** more words\n\n<!-- Markdeep: --><script src=\"markdeep.min.js\"></script>\n",
        "[a]:  \n/url extra words\n",                         // definition across a hard break
        "[label  \ntwo]: /url extra words\n",                 // label across a hard break
        "12345678 12345678 12345 12345678 12345678 12345 ab <urn:isbn:045145052345>\n", // autolink
        "12345678 12345678 12345 12345678 12345678 12345 ab <tel:1-800-555-0199-00>\n",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_all(&cases, 6..=30);
}

#[test]
fn hard_breaks_round_trip() {
    let cases: Vec<String> = [
        "line one with several words  \nline two with several more words\n",
        "line one with several words\\\nline two with several more words\n",
        "- item one has words  \n  continued after a two-space break\n",
        "> quoted words here  \n> more quoted words\n",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_all(&cases, 8..=40);
}

#[test]
fn tables_round_trip() {
    let cases: Vec<String> = [
        "| a | b |\n| - | - |\n| 1 | 2 | 3 |\n",
        "| a | b |\n|:-|-:|\n| 1 |\n| x | y |\n",
        "| a | b |\n|---|---|\n| `x` | \\| |\n",
        "| a | b |\n|---|---|\n| `x|y` | 2 |\n",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_all(&cases, [72].into_iter());
}

fn strings(cases: &[&str]) -> Vec<String> {
    cases.iter().map(|s| s.to_string()).collect()
}

/// Container content (indented code, nested lists, fenced code, tables) must keep its meaning
/// under every hanging-indent configuration.
#[test]
fn containers_round_trip_with_hanging_indents() {
    let cases = strings(&[
        "- item with some words to wrap\n\n      indented code   kept\n\n  after code\n",
        "1. item with some words to wrap\n\n       indented code\n",
        "10. item with some words to wrap\n    - nested item with words\n    - another nested\n",
        "- item\n\n  ```\n  fenced   code\n  ```\n\n  | a | b |\n  | - | - |\n  | 1 | 2 |\n",
        "- [ ] task item with some words to wrap around\n\n      code under task\n",
        "[^1]: footnote with some words to wrap around\n\n        footnote code\n",
    ]);
    for exact in [None, Some(2), Some(4), Some(6)] {
        for minimum in [None, Some(2), Some(4), Some(8)] {
            let mut base = FormattingOptions::default();
            base.hanging_indent = exact;
            base.min_hanging_indent = minimum;
            assert_all_with(&cases, &base, (8..=40).step_by(4));
        }
    }
}

#[test]
fn wrap_modes_round_trip() {
    let mut rng = Lcg(0xc0ffee);
    let mut cases = Vec::new();
    for _ in 0..60 {
        let a = random_paragraph(&mut rng);
        let b = random_paragraph(&mut rng);
        // Multi-line source paragraphs exercise `preserve`; sentence ends exercise `sentence`.
        cases.push(format!(
            "{a}. Next {b}\nSecond line. Third sentence here.\n"
        ));
        cases.push(format!("- {a}\n  {b}. More words\n"));
        cases.push(format!("> {a}  \n> {b}\n"));
    }
    for wrap in [
        WrapMode::Always,
        WrapMode::Never,
        WrapMode::Preserve,
        WrapMode::Sentence,
    ] {
        let mut base = FormattingOptions::default();
        base.wrap = wrap;
        assert_all_with(&cases, &base, (8..=40).step_by(8));
    }
}

#[test]
fn ordered_list_styles_round_trip() {
    let cases = strings(&[
        "1. one\n1. two\n1. three\n",
        "7. seven with words to wrap around\n8. eight\n9. nine with words to wrap\n   continued\n\n   second paragraph of nine\n9. ten after a width change\n\n       indented code\n",
        "3) a\n9) b\n   1. nested one\n   5. nested two\n2) c\n",
        "0. zero start\n5. five\n",
    ]);
    for style in [
        OrderedListStyle::Preserve,
        OrderedListStyle::Ascending,
        OrderedListStyle::One,
    ] {
        let mut base = FormattingOptions::default();
        base.ordered_lists = style;
        assert_all_with(&cases, &base, (8..=40).step_by(8));
    }
}
