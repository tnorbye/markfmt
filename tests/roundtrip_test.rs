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
    "#123",
    ";semi",
    "diff",
    "-u",
    "Index:",
    ":u",
    "Change-Id:",
    "Bug:",
    "Signed-off-by:",
    "(cherry",
    "JJ:note",
    "https://e.x/path",
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
        "alpha beta gamma Index: foo bar baz qux\n",
        "Start :u alpha beta gamma delta\n",
        "Index:\tfoo\n\nEnd.\n",
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

fn run_git_pipe(args: &[&str], input: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("git")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn git");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().replace("\r\n", "\n")
}

fn check_commit_git_invariants(input: &str, opts: &FormattingOptions) -> Option<String> {
    if let Some(failure) = check_round_trip(input, opts) {
        return Some(failure);
    }
    let once = markfmt::format_markdown(input, opts);
    let comment_cfg = format!("core.commentString={}", opts.comment_char);
    let separators_cfg = format!("trailer.separators={}", opts.trailer_separators);

    let strip_before = run_git_pipe(&["-c", &comment_cfg, "stripspace", "-s"], input);
    let strip_after = run_git_pipe(&["-c", &comment_cfg, "stripspace", "-s"], &once);
    let words_before: Vec<&str> = strip_before.split_whitespace().collect();
    let words_after: Vec<&str> = strip_after.split_whitespace().collect();
    if words_before != words_after || common::render(&strip_before) != common::render(&strip_after)
    {
        return Some(format!(
            "git stripspace -s changed content with {opts:?}\ninput: {input:?}\nonce: {once:?}\nstripspace before: {strip_before:?}\nstripspace after: {strip_after:?}"
        ));
    }

    // The trailer keys the options were given are the ones Git has configured.
    let mut trailer_args = vec!["-c".to_string(), comment_cfg.clone()];
    trailer_args.extend(["-c".to_string(), separators_cfg]);
    if let [name, key] = &opts.trailer_keys[..] {
        trailer_args.extend(["-c".to_string(), format!("trailer.{name}.key={key}")]);
    }
    trailer_args.extend(["interpret-trailers".to_string(), "--parse".to_string()]);
    let trailer_args: Vec<&str> = trailer_args.iter().map(String::as_str).collect();
    let trailers_before = run_git_pipe(&trailer_args, input);
    let trailers_after = run_git_pipe(&trailer_args, &once);
    if trailers_before != trailers_after {
        return Some(format!(
            "git interpret-trailers --parse changed with {opts:?}\ninput: {input:?}\nonce: {once:?}\ntrailers before: {trailers_before:?}\ntrailers after: {trailers_after:?}"
        ));
    }

    None
}

#[test]
fn commit_messages_round_trip_and_preserve_git_stripspace_and_trailers() {
    let mut rng = Lcg(0x6170_c0de);
    let mut cases: Vec<String> = [
        "Subject\n\n<rules>\n\nJJ: note\n",
        "Subject\n\n</rules>\n\nJJ: note\n",
        "Subject\n\n<rules>\n\n# comment\n",
        "Subject\n\n- heading\n  ===\n",
        "Subject\n\n> heading\n> ===\n",
        "Subject\n\n| a | b |\n| - | - |\n| 1 | 2 |\n",
        "Subject\n\na | b\n- | -\n1 | 2\n",
        "Subject\n\nalpha beta gamma Index: foo bar baz qux\n\nEnd.\n",
        "Subject\n\nStart :u alpha beta gamma delta\n\nEnd.\n",
        "Subject line kept verbatim even when long\n\nFixes issue #123 and runs diff -u and Index: check.\n\nBug: 123\nChange-Id: I0123456789abcdef0123456789abcdef01234567\n# Please enter the commit message\n#\tmodified:   src/lib.rs\n",
        "* Bullet-shaped subject line kept verbatim\n\nhttps://e.x\nwords on line two\n",
        "1. Numbered subject line\n\nNote: two-line prose paragraph\nthat shrinks at wide widths.\n",
        "Subject\n\np1\np2\np3\nSigned-off-by: A <a@b.c>\n",
        "Subject\n\np1\np2\np3\np4\nSigned-off-by: A <a@b.c>\n",
        "Subject\n\nProse line without blank line before trailer.\nChange-Id: I0123456789abcdef0123456789abcdef01234567\n",
        "Subject\n\n  indented first\nBug: 123\n",
        "Subject\n\nBug: 123\n  indented continuation\n(cherry picked from commit 0123456789abcdef0123456789abcdef01234567)\nChange-Id: I0123456789abcdef0123456789abcdef01234567\n",
        "Subject\n\nSee token : spaced colon and diff -r flags in prose.\n",
        "Subject\n\n #123 indented hash words wrap here\n",
        "Subject\n\nLine one  \n#foo comment after a hard break\nmore text here\n",
        "Subject\n\nA\\\n  diff -u and Bug: words\n",
        "Subject\n\n- parent item\n#comment\n  - child item with words\n",
        "Subject\n\n- parent item\n\n#comment\n\n  continuation paragraph of the item\n",
        "Subject\n\nfoo `one   two\n#end` tail words\n",
        "Subject\n\nBody text.\n\nCustom: a long trailer value that exceeds narrow widths\n---\nnotes after the divider\n",
        "Subject\n\nSome prose before the divider\n---\nmore\n",
        "Subject\n\nBody.\n\nReviewed-by=Some Person With A Long Name <p@e.x>\n",
        "Subject\n\nBody.\n\n//  ------------------------ >8 ------------------------\n// Do not modify.\ndiff --git a/x b/x\n+a  b   c\n",
        "Subject\n\nhttps://example.com\nwords on line two\n\n---\nnotes after the divider\n",
        "Subject\n\n #123 code `one   two\n#end` tail\n",
        "Subject\n\nfoo `one   two\n--- content` tail words\n",
        "Subject\n\nBody.\n\nThis is one long prose line that will wrap into several lines at a narrow width\nReviewed-by: Some Reviewer\n",
        "Subject\n\n- parent\n#comment\n  - child\nSigned-off-by: Alice Example <alice@example.com>\n",
        "Subject\n\n- first  \n#foo\nsecond\n",
        "Subject\n\n`one   two\n# comment\n`\n",
        "Subject\n\n> quoted   text\n#foo\n> more   quoted\n",
        "Subject\n\n`one   two\n<rules>\n# comment\n`\n",
        "Subject\n\n#comment\n<pre>\ncode\n\n\n",
        "Subject\n\nEarlier   paragraph.\n\n`one   two\n<rules>\n# comment\n`\n",
        "Subject\n\nAlpha beta // comment deleted words more.\n",
        "Subject\n\nOne two three four // comment deleted words more.\n",
        "Subject\n\n // comment indented words more.\n",
        "Subject\n\n//\ncomment\n",
        "Subject\n\n  # Indented heading\n",
        "Subject\n\n#   comment heading\n",
        "Subject\n\n- first\n-   comment item with words\n",
        "Subject\n\nhi\n----\n\nBug: 123\n",
        "Subject\n\n  ---\n\nBug: 123\n",
        "Subject\n\n-   item\n\n    second paragraph\n  ---\n",
        "Subject\n\n-   item\n\n    second paragraph\n  | a | b |\n  | - | - |\n",
        "Subject\n\n- #123 words\n\n  second paragraph\n",
        "Subject\n\n- first  \n  #123 words\n",
        "Subject\n\n- item\n<rules>\n  ---\n</rules>\n",
        "Subject\n\n-   item\n<rules>\n  > quote\n</rules>\n",
        "Subject\n\n```\ncode\n```\n-----BEGIN PGP SIGNATURE-----\niQEzBAABCAAdFiEE1234\n===\n-----END PGP SIGNATURE-----\n",
        "Subject\n\n```\ncode\n```\n-----BEGIN PGP SIGNATURE-----\niQEzBAABCAAdFiEE1234\n-----END PGP SIGNATURE-----\n    continuation\nfollowing text\n",
        "Subject\n\n-----BEGIN PGP SIGNATURE-----\nVersion: GnuPG v2\n\niQEzBAABCAAdFiEE1234\n-----END PGP SIGNATURE-----\n    continuation\nfollowing text\n",
        "Fix <code> tag handling\n\nBody   with   spaces.\n",
        "\nLong opening subject line after a blank line\n\nBody   text.\n",
        "# Template comment\n\nLong opening subject line after a comment\n\nBody   text.\n",
        // Git's trailer block ends before a blank-separated comment block (above a `---` divider, at
        // the end, or before a scissors line), so the trailers above it are found and kept.
        "Subject\n\nBody text.\n\nCustom: a long trailer value that exceeds narrow widths\n\n# a comment\n---\nnotes\n",
        "Subject\n\nBody text.\n\nCustom: a long trailer value that exceeds narrow widths\n\n# one\n\n# two\n",
        "Subject\n\nBody text.\n\nCustom: a long trailer value that exceeds narrow widths\n# adjacent comment\nOther: value\n",
        "Subject\n\nBody text.\n\nCustom: a long trailer value that exceeds narrow widths\n\n# comment\n# ------------------------ >8 ------------------------\ndiff --git a/x b/x\n",
        // Joining these would create a trailer Git did not see before.
        "Subject\n\nBody text.\n\nSigned-off-by\n: Alice <a@example.com>\n\n# a comment\n---\nnotes\n",
        "Subject\n\nBody text.\n\nSigned-off-by\n: Alice <a@example.com>\n\n# one\n\n# two\n",
        // Joining `diff` and `-u` at column 0 would start a `git am` patch.
        "Subject\n\ndiff\n-u foo bar baz\n",
        "Subject\n\nCustom\n: value words here\n\nmore text\n",
        // A lone or comment-led armored block stays whole.
        "-----BEGIN PGP SIGNATURE-----\n\niQEzBAABCAAdFiEE1234 long armor line words\nabc\n=XyZ1\n-----END PGP SIGNATURE-----\n",
        "# c\n-----BEGIN PGP SIGNATURE-----\n\niQEzBAABCAAdFiEE1234 long armor line words\nabc\n-----END PGP SIGNATURE-----\n",
        "# c\n-----BEGIN PGP SIGNATURE-----\nVersion: GnuPG v2\n\niQEzBAABCAAdFiEE1234\n-----END PGP SIGNATURE-----\n    continuation\nfollowing text\n",
        // Old-style `Conflicts:` footers, which Git skips when finding trailers.
        "Subject\n\nCustom: value\n\nConflicts:\n\tfoo.md\n",
        "Subject\n\nCustom: value\nConflicts:\n\tfoo.md\n",
        "Subject\n\nBody   text.\n\nConflicts:\n\tsome/long/path/that/goes/past/narrow/widths/file.md\n\tother.md\n# comment\n",
        // Collapsing whitespace must not create a `git am` patch break.
        "Subject\n\nBody.\n\ndiff  -u foo bar baz\n",
        "Subject\n\nBody.\n\ndiff\t-u foo bar baz\n",
        "Subject\n\nIndex:\tfoo\n\nmore text\n",
        // Also not under `--wrap preserve`, or in a two-line setext heading.
        "Subject\n\nFirst line.\ndiff  -u a b\nMore text after.\n",
        "Subject\n\nFirst line.\nIndex:\tfoo bar\nMore text after.\n",
        "Subject\n\n diff -u a b\n============\n\nMore text after.\n",
        "Subject\n\ndiff  -u a b\n=============\n\nMore text after.\n",
        "Subject\n\n Index: foo\n============\n\nMore text after.\n",
        // With a comment prefix ending in a space (`// `), `//\t` is not a comment until the
        // tab becomes a space.
        "Subject\n\n//\tlong text after a tab that wraps at narrow widths\n",
        // Blank lines added or removed around blocks must not change Git's trailers.
        "Subject\n```\ncode\n```\nCustom: value\n",
        "\nBug: 42\n",
        "\n\nBug: 42\n\n# comment\n",
        // A footer attached to a block CommonMark is still reading keeps that block as written.
        "Subject\n\n<rules>\n-   item\n\n    second paragraph\nConflicts:\n\tfile\n",
        "Subject\n\nBody   text   here.\nConflicts:\n\tfile\n",
        // Git reads `JJ:` lines as text (here, trailers), not comments.
        "\nJJ: note\n\tcode\nOther: value\n",
        "Subject\n\nBody.\n\nJJ: note\nOther: value\n",
        // Dropping the blank line would make the next pass read frontmatter.
        "\n---\nk: v\n#c\n---\nend\n",
        "\n\n---\nk: v\n---\nend   text\n",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let fixed_count = cases.len();

    for i in 0..80 {
        let p1 = random_paragraph(&mut rng);
        let p2 = random_paragraph(&mut rng);
        let footer = match i % 4 {
            0 => "\nBug: 42\nChange-Id: I0123456789abcdef0123456789abcdef01234567\n# comment\n",
            1 => "\nSigned-off-by: Dev <dev@example.com>\n",
            2 => "",
            _ => "\nNote: single trailer-shaped footer line here\n",
        };
        cases.push(format!("Commit subject {i}\n\n{p1}\n\n{p2}{footer}\n"));
    }

    // Pure-Rust round-trip across narrow-to-normal widths, wrap modes, and hanging indents.
    for (comment_char, separators) in [
        ("#", ":"),
        ("#", "-:"),
        ("#", "-"),
        (";", ":"),
        ("|", ":"),
        (">", ":"),
        ("// ", ":="),
        ("// comment", ":"),
        ("# comment", ":"),
        ("- comment", ":"),
    ] {
        for wrap in [
            WrapMode::Always,
            WrapMode::Never,
            WrapMode::Preserve,
            WrapMode::Sentence,
        ] {
            for hanging_indent in [None, Some(0)] {
                let mut base = FormattingOptions::default();
                base.commit_message = true;
                base.comment_char = comment_char.to_string();
                base.trailer_separators = separators.to_string();
                base.wrap = wrap;
                base.hanging_indent = hanging_indent;
                assert_all_with(&cases, &base, (10..=42).step_by(8));
            }
        }
    }

    // Executable Git subprocess invariants (`git stripspace -s` and `git interpret-trailers --parse`).
    let mut git_checks: Vec<(String, FormattingOptions)> = Vec::new();
    for case in &cases[..fixed_count + 20] {
        for (width, comment_char, separators, review_key) in [
            (16, "#", ":", false),
            (17, "#", "-", false),
            (10, "#", "-:", false),
            (30, "#", ":", false),
            (72, "#", ":", false),
            (20, ";", ":", false),
            (20, "// ", ":", false),
            (20, "// comment", ":", false),
            (20, "# comment", ":", false),
            (20, "- comment", ":", false),
            (20, "#", ":=", false),
            (25, "#", ":", true),
        ] {
            let mut opts = FormattingOptions::default();
            opts.commit_message = true;
            opts.max_line_width = width;
            opts.comment_char = comment_char.to_string();
            opts.trailer_separators = separators.to_string();
            if review_key {
                opts.trailer_keys = vec!["review".to_string(), "Reviewed-by".to_string()];
            }
            git_checks.push((case.clone(), opts));
        }
    }

    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(git_checks.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failures: Vec<String> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Vec::new();
                    loop {
                        let idx = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((case, opts)) = git_checks.get(idx) else {
                            break;
                        };
                        if let Some(err) = check_commit_git_invariants(case, opts) {
                            local.push(err);
                        }
                    }
                    local
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    });
    assert!(
        failures.is_empty(),
        "{} git invariant checks failed; first:\n\n{}",
        failures.len(),
        failures[0]
    );
}

#[test]
fn commit_trailer_fallback_with_custom_comment_prefix_round_trips() {
    // These tails use `>` as Git's comment prefix; under another prefix, the same lines
    // would be Markdown blockquotes rather than a detached comment/scissors tail.
    let head = "S\n  # h\ndiff -u\n  > q\nSigned-off-by: A\n";
    for tail in [
        "> comment\n\n",
        "> ------------------------ >8 ------------------------\n+x\n\n",
        "JJ: ignore-rest\n+x\n\n",
    ] {
        let input = format!("{head}{tail}");
        for wrap in [
            WrapMode::Always,
            WrapMode::Never,
            WrapMode::Preserve,
            WrapMode::Sentence,
        ] {
            for width in [10, 20, 72] {
                let mut opts = FormattingOptions::default();
                opts.commit_message = true;
                opts.comment_char = ">".to_string();
                opts.max_line_width = width;
                opts.wrap = wrap;
                if let Some(failure) = check_commit_git_invariants(&input, &opts) {
                    panic!("{failure}");
                }
            }
        }
    }
}
