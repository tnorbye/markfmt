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

use pretty_assertions::assert_eq;

use markfmt::{format_markdown, FormattingOptions};

#[test]
fn keeps_short_multi_word_link_unbroken_across_lines() {
    let input = "For additional details please read the [Rust programming language](https://www.rust-lang.org) guide.\n";
    let mut opts = FormattingOptions::default();
    opts.max_line_width = 60;
    let out = format_markdown(input, &opts);
    assert!(
        out.contains("[Rust programming language](https://www.rust-lang.org)"),
        "link was unexpectedly broken across lines:\n{out}"
    );
    assert_eq!(format_markdown(&out, &opts), out);
}

#[test]
fn wraps_very_long_link_text_inside_brackets_keeping_last_word_and_url_atomic() {
    let input = "Read the comprehensive [architecture and implementation specification document for this module](https://example.com/docs/architecture/spec.html) carefully.\n";
    let mut opts = FormattingOptions::default();
    opts.max_line_width = 60;
    let out = format_markdown(input, &opts);
    // Must wrap inside `[...]` so it doesn't overflow horizontally while keeping `module](url)` unbroken.
    assert!(out.lines().count() >= 2);
    assert!(out.contains("module](https://example.com/docs/architecture/spec.html)"));
    assert_eq!(format_markdown(&out, &opts), out);
}

#[test]
fn unencoded_space_containing_links_stay_verbatim_and_unbroken() {
    let input = concat!(
        "See the screenshot at [my picture](file://My Files/foo.png) and the ",
        "diagram ![architecture](./My Docs/arch v2.png) plus [report](My Report.pdf) ",
        "for full visual context.\n",
    );
    let mut opts = FormattingOptions::default();
    opts.max_line_width = 55;
    let out = format_markdown(input, &opts);

    assert!(
        out.contains("[my picture](file://My Files/foo.png)"),
        "space-containing file:// link broken or mutated:\n{out}"
    );
    assert!(
        out.contains("![architecture](./My Docs/arch v2.png)"),
        "space-containing relative image link broken or mutated:\n{out}"
    );
    assert!(
        out.contains("[report](My Report.pdf)"),
        "space-containing doc link broken or mutated:\n{out}"
    );
    assert!(
        !out.contains("%20"),
        "must never rewrite spaces to %20:\n{out}"
    );
    assert_eq!(format_markdown(&out, &opts), out);
}

#[test]
fn at_include_wikilink_inline_code_and_math_handling() {
    let input = concat!(
        "Review @[Architecture Guide](./docs/arch.md) and [[Project Wiki Page]] ",
        "alongside `let total = alpha + beta;` plus inline formula $a + b = c$.\n",
        "\n",
        "Budget range is $100 to $200 or $50 -$60 across departments.\n",
    );
    let mut opts = FormattingOptions::default();
    opts.max_line_width = 48;
    let out = format_markdown(input, &opts);

    assert!(
        out.contains("@[Architecture Guide](./docs/arch.md)"),
        "broken @[...](...):\n{out}"
    );
    assert!(
        out.contains("[[Project Wiki Page]]"),
        "broken wikilink:\n{out}"
    );
    assert!(
        out.contains("`let total = alpha + beta;`"),
        "broken inline code:\n{out}"
    );
    assert!(out.contains("$a + b = c$"), "broken inline math:\n{out}");
    assert_eq!(format_markdown(&out, &opts), out);
}

#[test]
fn link_reference_definitions_preserved_verbatim() {
    let input = concat!(
        "Read [the guide][guide] and [notes].\n",
        "\n",
        "[guide]: https://example.com/guide   \"User Guide\"\n",
        "[notes]: https://example.com/notes\n",
    );
    let opts = FormattingOptions::default();
    let out = format_markdown(input, &opts);
    assert_eq!(out, input);
}
