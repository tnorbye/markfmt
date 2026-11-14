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

mod common;

use pretty_assertions::assert_eq;

use markfmt::{format_markdown, FormattingOptions};

#[test]
fn lists_preserve_markers_tightness_and_task_checkboxes() {
    let input = concat!(
        "* First bullet item that is quite long and should wrap cleanly onto a second aligned continuation line.\n",
        "* Second tight bullet\n",
        "  - Nested item A\n",
        "  - Nested item B\n",
        "\n",
        "10. Tenth item with extra prose that wraps cleanly with four-space continuation indentation.\n",
        "11) [x] Completed task item that wraps cleanly while keeping its checkbox intact on line one.\n",
    );
    let opts = common::options(60);
    let expected = concat!(
        "* First bullet item that is quite long and should wrap\n",
        "  cleanly onto a second aligned continuation line.\n",
        "* Second tight bullet\n",
        "  - Nested item A\n",
        "  - Nested item B\n",
        "\n",
        "10. Tenth item with extra prose that wraps cleanly with\n",
        "    four-space continuation indentation.\n",
        "11) [x] Completed task item that wraps cleanly while keeping\n",
        "    its checkbox intact on line one.\n",
    );
    common::assert_format(input, expected, &opts);
}

#[test]
fn loose_list_and_hanging_indent_override() {
    let input = concat!(
        "- Item one first paragraph that wraps across two lines at width 45.\n",
        "\n",
        "  Item one second paragraph.\n",
        "\n",
        "- Item two.\n",
    );
    let mut opts = common::options(45);
    opts.hanging_indent = Some(4);
    let expected = concat!(
        "- Item one first paragraph that wraps across\n",
        "    two lines at width 45.\n",
        "\n",
        "    Item one second paragraph.\n",
        "\n",
        "- Item two.\n",
    );
    common::assert_format(input, expected, &opts);
}

#[test]
fn four_line_list_item_uses_first_width_and_rest_width_optimally() {
    let input = "- Word1 word2 word3 word4 word5 word6 word7 word8 word9 word10 word11 word12 word13 word14 word15 word16 word17 word18 word19.\n";
    let mut opts = common::options(36);
    opts.optimal = true;
    let out = format_markdown(input, &opts);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.len() >= 4);
    assert!(lines[0].starts_with("- "));
    for line in &lines {
        assert!(line.len() <= 36, "list line overflowed: {line:?}");
    }
    for line in &lines[1..] {
        assert!(line.starts_with("  "), "missing 2-space indent: {line:?}");
    }
    assert_eq!(format_markdown(&out, &opts), out);
}

#[test]
fn blockquotes_lazy_continuation_and_github_alerts() {
    let input = concat!(
        "> [!NOTE]\n",
        "> Useful information that users should know even when skimming long paragraphs that exceed seventy-two columns.\n",
        "\n",
        "> Outer quote line\n",
        "lazy continuation joined cleanly.\n",
        ">\n",
        "> > Nested quote with    extra spaces.\n",
    );
    let opts = common::options(60);
    let expected = concat!(
        "> [!NOTE]\n",
        "> Useful information that users should know even when\n",
        "> skimming long paragraphs that exceed seventy-two columns.\n",
        "\n",
        "> Outer quote line lazy continuation joined cleanly.\n",
        ">\n",
        "> > Nested quote with extra spaces.\n",
    );
    common::assert_format(input, expected, &opts);
}

#[test]
fn mermaid_and_code_blocks_remain_byte_for_byte_untouched() {
    let input = concat!(
        "Paragraph    with   extra   spaces.\n",
        "\n",
        "```mermaid\n",
        "flowchart TD\n",
        "    A[Extremely long node label that must definitely NOT be wrapped at 72 chars] --> B{Decision}\n",
        "    B -->|Yes| C[Keep exact spacing]\n",
        "```\n",
        "\n",
        "    indented   code   line\n",
        "      kept verbatim\n",
    );
    let opts = FormattingOptions::default();
    let out = format_markdown(input, &opts);
    let expected = concat!(
        "Paragraph with extra spaces.\n",
        "\n",
        "```mermaid\n",
        "flowchart TD\n",
        "    A[Extremely long node label that must definitely NOT be wrapped at 72 chars] --> B{Decision}\n",
        "    B -->|Yes| C[Keep exact spacing]\n",
        "```\n",
        "\n",
        "    indented   code   line\n",
        "      kept verbatim\n",
    );
    assert_eq!(out, expected);
}

#[test]
fn gfm_table_alignment_unicode_escaped_pipes_and_disable_flag() {
    let input = concat!(
        "|Name|Code `a\\|b`|Count|\n",
        "|:---|:---:|---:|\n",
        "|日本語|x \\| y|42|\n",
        "|Alpha|`m`|7|\n",
    );
    let out = format_markdown(input, &FormattingOptions::default());
    let expected = concat!(
        "| Name   | Code `a\\|b` | Count |\n",
        "| :----- | :---------: | ----: |\n",
        "| 日本語 |   x \\| y    |    42 |\n",
        "| Alpha  |     `m`     |     7 |\n",
    );
    assert_eq!(out, expected);

    let mut unaligned_opts = FormattingOptions::default();
    unaligned_opts.align_table_columns = false;
    let unaligned = format_markdown(input, &unaligned_opts);
    assert_eq!(unaligned, input);
}

#[test]
fn headings_footnotes_and_ignore_comments() {
    let input = concat!(
        "#   ATX   Heading  `keep  spaces`  ##\n",
        "\n",
        "Setext   Heading\n",
        "===\n",
        "\n",
        "<!-- markfmt:off -->\n",
        "Do   not   touch   this   unwrapped   paragraph   even   when   extremely   long.\n",
        "<!-- markfmt:on -->\n",
        "\n",
        "<!-- prettier-ignore -->\n",
        "Also   untouched   next   block.\n",
        "\n",
        "<!-- markfmt-ignore-next -->\n",
        "Untouched   next   block   too.\n",
        "\n",
        "Note ref[^1].\n",
        "\n",
        "[^1]: Footnote body that wraps cleanly at sixty columns across two aligned lines.\n",
    );
    let opts = common::options(60);
    let expected = concat!(
        "# ATX Heading `keep  spaces` ##\n",
        "\n",
        "Setext Heading\n",
        "==============\n",
        "\n",
        "<!-- markfmt:off -->\n",
        "Do   not   touch   this   unwrapped   paragraph   even   when   extremely   long.\n",
        "<!-- markfmt:on -->\n",
        "\n",
        "<!-- prettier-ignore -->\n",
        "Also   untouched   next   block.\n",
        "\n",
        "<!-- markfmt-ignore-next -->\n",
        "Untouched   next   block   too.\n",
        "\n",
        "Note ref[^1].\n",
        "\n",
        "[^1]: Footnote body that wraps cleanly at sixty columns\n",
        "    across two aligned lines.\n",
    );
    common::assert_format(input, expected, &opts);
}

#[test]
fn editorconfig_indent_is_a_minimum_but_explicit_hanging_indent_is_exact() {
    let input = "1. one two three four five six seven\n";
    let mut from_editorconfig = common::options(20);
    from_editorconfig.min_hanging_indent = Some(2);
    assert_eq!(
        format_markdown(input, &from_editorconfig),
        "1. one two three\n   four five six\n   seven\n"
    );

    let mut explicit = common::options(20);
    explicit.hanging_indent = Some(2);
    assert_eq!(
        format_markdown(input, &explicit),
        "1. one two three\n  four five six\n  seven\n"
    );
}
