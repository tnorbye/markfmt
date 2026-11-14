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
fn markdeep_auto_detection_header_diagrams_math_admonitions_definitions_and_footer() {
    let input = concat!(
        "         **Markdeep Document Title**\n",
        "             Alice Example, 2026\n",
        "\n",
        "This introductory paragraph has way too many words on a single line so it should be wrapped cleanly at 55 columns.\n",
        "\n",
        "*************************************\n",
        "*  Client  --->  Server  --->  DB   *\n",
        "*************************************\n",
        "[Figure 1: System request flow diagram]\n",
        "\n",
        "!!! warning: Important Caveat\n",
        "    Be careful because this admonition paragraph also has many words that should wrap inside its four-space indent.\n",
        "\n",
        "Term One\n",
        ":   Definition body that runs quite long horizontally and needs to wrap neatly under the hanging four-space indent.\n",
        "\n",
        "$$\n",
        "x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}\n",
        "$$\n",
        "\n",
        "<style class=\"fallback\">body{visibility:hidden}</style><script>markdeepOptions={tocStyle:'long'};</script>\n",
        "<!-- Markdeep: --><style class=\"fallback\">body{visibility:hidden}</style><script src=\"markdeep.min.js\"></script>\n",
    );

    let opts = common::options(55);
    let expected = concat!(
        "         **Markdeep Document Title**\n",
        "             Alice Example, 2026\n",
        "\n",
        "This introductory paragraph has way too many words on a\n",
        "single line so it should be wrapped cleanly at 55\n",
        "columns.\n",
        "\n",
        "*************************************\n",
        "*  Client  --->  Server  --->  DB   *\n",
        "*************************************\n",
        "[Figure 1: System request flow diagram]\n",
        "\n",
        "!!! warning: Important Caveat\n",
        "    Be careful because this admonition paragraph also\n",
        "    has many words that should wrap inside its\n",
        "    four-space indent.\n",
        "\n",
        "Term One\n",
        ":   Definition body that runs quite long horizontally\n",
        "    and needs to wrap neatly under the hanging\n",
        "    four-space indent.\n",
        "\n",
        "$$\n",
        "x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}\n",
        "$$\n",
        "\n",
        "<style class=\"fallback\">body{visibility:hidden}</style><script>markdeepOptions={tocStyle:'long'};</script>\n",
        "<!-- Markdeep: --><style class=\"fallback\">body{visibility:hidden}</style><script src=\"markdeep.min.js\"></script>\n",
    );
    common::assert_format(input, expected, &opts);
}

#[test]
fn explicit_markdeep_option_works_for_md_html_without_footer() {
    let input = concat!(
        "**Standalone Header**\n",
        "  Author Line\n",
        "\n",
        "!!! tip\n",
        "    Short tip that wraps cleanly at width 40 across lines.\n",
    );
    let mut opts = common::options(40);
    opts.markdeep = Some(true);
    let out = format_markdown(input, &opts);
    let expected = concat!(
        "**Standalone Header**\n",
        "  Author Line\n",
        "\n",
        "!!! tip\n",
        "    Short tip that wraps cleanly at\n",
        "    width 40 across lines.\n",
    );
    assert_eq!(out, expected);
}

#[test]
fn inline_code_mentioning_markdeep_does_not_corrupt_later_fences() {
    let input = concat!(
        "### Markdeep Notes\n",
        "\n",
        "Files containing `markdeep.min.js` /\n",
        "`<!-- Markdeep:` markers enable Markdeep mode automatically when active.\n",
        "\n",
        "```markdown\n",
        "<!-- markfmt:off -->\n",
        "Custom   spacing   kept   verbatim   inside   fence.\n",
        "<!-- markfmt:on -->\n",
        "```\n",
    );
    let opts = FormattingOptions::default();
    let expected = concat!(
        "### Markdeep Notes\n",
        "\n",
        "Files containing `markdeep.min.js` / `<!-- Markdeep:` markers enable\n",
        "Markdeep mode automatically when active.\n",
        "\n",
        "```markdown\n",
        "<!-- markfmt:off -->\n",
        "Custom   spacing   kept   verbatim   inside   fence.\n",
        "<!-- markfmt:on -->\n",
        "```\n",
    );
    common::assert_format(input, expected, &opts);
}

#[test]
fn markdeep_mode_reflows_prose_mentioning_marker_while_preserving_real_footer() {
    let input = concat!(
        "This Markdeep paragraph mentions `<!-- Markdeep:` in inline code and has extra   spaces.\n",
        "\n",
        "<!-- Markdeep: --><style class=\"fallback\">body{visibility:hidden}</style><script src=\"markdeep.min.js\"></script>\n",
    );
    let mut opts = common::options(60);
    opts.markdeep = Some(true);
    let expected = concat!(
        "This Markdeep paragraph mentions `<!-- Markdeep:` in inline\n",
        "code and has extra spaces.\n",
        "\n",
        "<!-- Markdeep: --><style class=\"fallback\">body{visibility:hidden}</style><script src=\"markdeep.min.js\"></script>\n",
    );
    common::assert_format(input, expected, &opts);
}

#[test]
fn inline_admonition_marker_does_not_become_a_block() {
    let input = "aaaaaaaaa !!!:Note bbbbbbbbb\n    continuation   text   here\n";
    let mut opts = FormattingOptions::default();
    opts.max_line_width = 15;
    opts.markdeep = Some(true);
    let output = format_markdown(input, &opts);
    assert!(
        !output.lines().any(|line| line.starts_with("!!!")),
        "{output}"
    );
    assert_eq!(format_markdown(&output, &opts), output);
}
