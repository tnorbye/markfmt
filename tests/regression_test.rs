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
use common::{
    assert_format, assert_round_trip, assert_round_trip_renderers_disagree, options, render,
};
use markfmt::WrapMode;

#[test]
fn parser_range_boundaries_keep_following_content() {
    for (input, expected) in [
        (
            "* Compile docs\n\n Publish the pages.\n",
            "* Compile docs\n\nPublish the pages.\n",
        ),
        (
            "+    Validate links\n\n  Keep the generated report.\n",
            "+ Validate links\n\nKeep the generated report.\n",
        ),
        (
            "1.\n  Following paragraph.\n",
            "1.\n\nFollowing paragraph.\n",
        ),
        (
            "[^build]: [^cache]: Reuse compiled dependencies.\n",
            "[^build]:\n\n[^cache]: Reuse compiled dependencies.\n",
        ),
    ] {
        assert_format(input, expected, &options(72));
        assert_round_trip(input, &options(72));
    }
}

#[test]
fn raw_text_html_in_list_keeps_a_single_following_blank_line() {
    for input in [
        "- <code>x</code>\n\nPara.\n",
        "- <pre>x</pre>\n\nPara.\n",
        "- a\n  - <code>x</code> y\n\nPara.\n",
        "1. <code>x</code>\n\n\nPara.\n",
    ] {
        let expected = input.replace("\n\n\n", "\n\n");
        assert_format(input, &expected, &options(72));
        assert_round_trip(input, &options(72));
    }
}

#[test]
fn footnote_text_survives_label_normalization() {
    // The parser normalizes labels; the definition's body must not be lost when the written
    // label differs from the normalized one. The marker is kept exactly as written.
    for (input, expected) in [
        (
            "Ref[^Foo  Bar].\n\n[^Foo  Bar]: Footnote text here.\n",
            "Ref[^Foo  Bar].\n\n[^Foo  Bar]: Footnote text here.\n",
        ),
        (
            "Ref[^foo].\n\n[^FOO]:   Footnote text here.\n",
            "Ref[^foo].\n\n[^FOO]: Footnote text here.\n",
        ),
        (
            "Ref[^a].\n\n[^ a]: Footnote text here.\n",
            "Ref[^a].\n\n[^ a]: Footnote text here.\n",
        ),
    ] {
        assert_format(input, expected, &options(72));
    }
    // Ambiguous labels stay verbatim so neither renderer gains a link reference definition.
    for input in [
        "Ref[^Foo  Bar].\n\n[^Foo  Bar]: Footnote text here.\n",
        "Ref[^a\tb].\n\n[^a\tb]: Footnote text here.\n",
        "Ref[^a].\n\n[^ a]: Footnote text here.\n",
        "Ref[^a\\]b].\n\n[^a\\]b]: Footnote text here.\n",
    ] {
        for width in [8, 12, 30, 72] {
            assert_round_trip(input, &options(width));
        }
    }
    assert_round_trip("Ref[^foo].\n\n[^FOO]: Footnote text here.\n", &options(8));
}

#[test]
fn checkbox_alone_on_its_line_keeps_the_following_blank_line() {
    for (input, expected) in [
        ("- [x]\n\n  Second.\n", "- [x]\n\n  Second.\n"),
        ("1. [ ]\n\n   Second.\n", "1. [ ]\n\n   Second.\n"),
        (
            "- [ ]\n\n  Second.\n\n- [x]\n",
            "- [ ]\n\n  Second.\n\n- [x]\n",
        ),
        // pulldown-cmark ends this list after the checkbox, but GFM keeps `Second.` in the item,
        // so the list is kept exactly as written.
        ("* [x]\n\n\n  Second.\n", "* [x]\n\n\n  Second.\n"),
    ] {
        assert_format(input, expected, &options(72));
        for width in [4, 12, 72] {
            assert_round_trip(input, &options(width));
        }
    }
}

#[test]
fn lazy_footnote_continuations_stay_paragraph_text() {
    // Indenting a lazy `===` into the footnote body would make it a setext underline.
    for (input, expected) in [
        ("R[^1].\n\n[^1]: a\n===\n", "R[^1].\n\n[^1]: a ===\n"),
        ("R[^1].\n\n[^1]: a\n  ===\n", "R[^1].\n\n[^1]: a ===\n"),
        (
            "R[^1].\n\n[^1]: a\n\n    b\n===\n",
            "R[^1].\n\n[^1]: a\n\n    b ===\n",
        ),
    ] {
        assert_format(input, expected, &options(72));
        for wrap in [
            WrapMode::Always,
            WrapMode::Never,
            WrapMode::Preserve,
            WrapMode::Sentence,
        ] {
            for width in [4, 12, 72] {
                let mut opts = options(width);
                opts.wrap = wrap;
                assert_round_trip(input, &opts);
            }
        }
    }
}

#[test]
fn literal_alert_markers_are_not_promoted_to_alerts() {
    for (input, expected) in [
        // Indented code, not an alert; it gets the usual blank line before the next block.
        (">     [!NOTE]\n> body\n", ">     [!NOTE]\n>\n> body\n"),
        (">     [!NOTE]\n", ">     [!NOTE]\n"),
        (
            "- >     [!NOTE]\n  > body\n",
            "- >     [!NOTE]\n  >\n  > body\n",
        ),
        // pulldown-cmark does not accept the extra space, so the line stays as written.
        (">  [!TIP]\n> body\n", ">  [!TIP]\n> body\n"),
    ] {
        assert_format(input, expected, &options(72));
        for width in [4, 12, 72] {
            assert_round_trip(input, &options(width));
        }
    }
}

#[test]
fn nested_fences_and_opaque_html_are_preserved() {
    for input in [
        "- ```\n  <rules>\n  a   b\n  </rules>\n  ```\n",
        "> ```\n> <rules>\n> a   b\n> </rules>\n> ```\n",
        "<pre>\n<rules>\nhello   there\n</rules>\n</pre>\n",
        "<script>\n<rules>\nhello   there\n</rules>\n</script>\n",
        "<!--\n<rules>\nhello   there\n</rules>\n-->\n",
        "```\ncode\n\n\n",
    ] {
        assert_format(input, input, &options(72));
        assert_round_trip(input, &options(72));
    }
}

#[test]
fn syntax_tabs_are_independent_of_editor_tab_width() {
    for tab_width in [2, 4, 8] {
        let mut opts = options(72);
        opts.tab_width = tab_width;
        for (input, expected) in [
            (">\tfoo\n", "> foo\n"),
            ("-\tfoo\n\tbar\n", "- foo bar\n"),
            ("- foo\n\n\t  code\n", "- foo\n\n      code\n"),
        ] {
            assert_format(input, expected, &opts);
            assert_round_trip(input, &opts);
        }
    }
}

#[test]
fn tight_lists_keep_adjacent_blocks_tight() {
    for input in [
        "- Run checks\n  > Keep diagnostics.\n- Publish docs\n",
        "+ # Build docs\n+ Run\n  ---\n  Check the examples.\n",
        "- Build docs\n  > Keep the cache.\n  ```sh\n  markfmt --check .\n  ```\n- Package\n",
    ] {
        // These documentation steps must stay tight even when an item contains other blocks.
        assert!(!render(input).contains("<li> <p>"));
        assert_format(input, input, &options(72));
        assert_round_trip(input, &options(72));
    }
}

#[test]
fn lazy_quotes_keep_markers_inline() {
    for (input, expected) in [
        (
            "> Documentation\nuses Markdown\n===\n",
            "> Documentation uses Markdown ===\n",
        ),
        (
            "> Supported operations\n    + validate docs\n",
            "> Supported operations + validate docs\n",
        ),
    ] {
        assert_format(input, expected, &options(72));
        assert_round_trip(input, &options(72));
    }
}

#[test]
fn setext_headings_and_inline_whitespace_keep_their_meaning() {
    for (input, expected) in [
        ("#hashtag\n===\n", "#hashtag\n========\n"),
        ("#not atx\n---\n", "#not atx\n--------\n"),
        ("one  \ntwo\n===\n", "one  \ntwo\n=====\n"),
        ("# [a](url \"one  two\")\n", "# [a](url \"one  two\")\n"),
        (
            "# <a title=\"one  two\">link</a>\n",
            "# <a title=\"one  two\">link</a>\n",
        ),
    ] {
        assert_format(input, expected, &options(72));
        assert_round_trip(input, &options(72));
    }
}

#[test]
fn literals_and_backslashes_keep_significant_whitespace() {
    for (input, expected) in [
        // A code span's line break stays where it is: renderers disagree about the leading
        // whitespace of the next line (cmark-gfm strips it, pulldown-cmark keeps it).
        ("Text `one  \n two` here\n", "Text `one  \n two` here\n"),
        ("Text `one\n\ttwo` here\n", "Text `one\n\ttwo` here\n"),
        ("alpha\\  \nbeta\n", "alpha\\  \nbeta\n"),
        ("alpha\\ \nbeta\n", "alpha\\ beta\n"),
    ] {
        assert_format(input, expected, &options(72));
        assert_round_trip(input, &options(72));
    }
}

#[test]
fn preserve_keeps_existing_safe_line_boundaries() {
    let mut opts = options(72);
    opts.wrap = WrapMode::Preserve;
    for input in [
        "First line\n2. ordinary continuation\n",
        "alpha\\ \nbeta\n",
        "- first\nsecond\n",
    ] {
        assert_format(input, input, &opts);
        assert_round_trip(input, &opts);
    }
}

#[test]
fn ignore_next_preserves_complete_loose_lists() {
    let input = "<!-- prettier-ignore -->\n- keep   first\n\n- keep   second\n\nFormat   this.\n";
    let expected = "<!-- prettier-ignore -->\n- keep   first\n\n- keep   second\n\nFormat this.\n";
    assert_format(input, expected, &options(72));
    assert_round_trip(input, &options(72));
}

#[test]
fn empty_task_checkboxes_keep_their_required_space() {
    // Without the space, pulldown-cmark renders `[ ]` as text, so it must not be added.
    for input in ["- [ ] \n", "- [x] \n", "* [X] \n", "- [ ]\n", "* [x]\n"] {
        assert_format(input, input, &options(72));
        assert_round_trip(input, &options(72));
    }
}

#[test]
fn render_comparison_preserves_literals_and_attributes() {
    assert_eq!(render("some  text\nhere"), render("some text here"));
    assert_ne!(render("`one  two`"), render("`one two`"));
    assert_ne!(
        render("[a](url \"one  two\")"),
        render("[a](url \"one two\")")
    );
    assert_ne!(render("<pre>one  two</pre>"), render("<pre>one two</pre>"));
}

#[test]
fn list_padding_keeps_following_code_outside_the_list() {
    for input in [
        " +    Build documentation\n\n     markfmt --check docs/\n",
        "7. Build documentation\n\n  8. Publish packages\n\n    9. Check release notes\n",
    ] {
        let html = render(input);
        assert!(
            html.contains("</ul> <pre><code>") || html.contains("</ol> <pre><code>"),
            "the command must be a top-level code block: {html}"
        );
        assert_format(input, input, &options(72));
        assert_round_trip(input, &options(72));
    }
}

#[test]
fn reference_definition_context_is_preserved() {
    for input in [
        "[manual]: ./manual.md\n+\n",
        "[cli]: ./usage.md\n    * dry-run\n",
        "[release]: /releases\n\"Release notes\" draft\n",
        "> [guide]: ./guide.md\n[api]: ./api.md\n[guide]\n",
        "[format]: /format\n   * apply\n\n[check]: /check\n    * inspect\n",
    ] {
        for width in [6, 8, 20, 72] {
            assert_round_trip(input, &options(width));
        }
    }
}

#[test]
fn lazy_lists_and_alerts_follow_original_soft_breaks() {
    for input in [
        "* Draft\n * Review\n  * Format\n   * Verify\n    * Publish\n",
        "> [!NOTE]\nRun the formatter before publishing.\n",
        "> [!WARNING]\n> Keep the build artifacts.\n> > [!TIP]\n> Reuse cached dependencies.\n",
        "Supported syntax\n    > quote marker\n===\n",
    ] {
        for width in [8, 20, 72] {
            assert_round_trip(input, &options(width));
        }
    }
    // pulldown-cmark reads `\` as a setext heading; cmark-gfm reads it as a lazy continuation
    // followed by an empty list item.
    for width in [8, 20, 72] {
        assert_round_trip_renderers_disagree(
            "# Release checklist\n\n* [ ]\t\t\n\\\n-\n",
            &options(width),
        );
    }
}

#[test]
fn context_sensitive_blocks_round_trip_in_all_wrap_modes() {
    let cases = [
        "_ _ _ _ placeholder\n",
        "[release]: /releases \"Release notes\" draft\n",
        "[release]: /releases\n\"Release notes\" draft\n",
        "[^build\ncache]: documentation text\n",
        "| Command | Purpose |\n| --- |\n| check |\n",
        "* Draft\n * Review\n  * Format\n   * Verify\n    * Publish\n",
        " +    Build documentation\n\n     markfmt --check docs/\n",
        "- Run checks\n  > Keep diagnostics.\n- Publish docs\n",
        "> Documentation\nuses Markdown\n===\n",
    ];
    for wrap in [
        WrapMode::Always,
        WrapMode::Never,
        WrapMode::Preserve,
        WrapMode::Sentence,
    ] {
        for optimal in [true, false] {
            for width in [6, 8, 20, 72] {
                let mut opts = options(width);
                opts.wrap = wrap;
                opts.optimal = optimal;
                for input in cases {
                    assert_round_trip(input, &opts);
                }
            }
        }
    }
}

#[test]
fn preserve_mode_keeps_indentation_that_prevents_new_blocks() {
    let mut opts = options(72);
    opts.wrap = WrapMode::Preserve;
    for input in [
        "Release notes\n    ***\n",
        "Supported syntax\n    ## heading marker\n",
        "CLI syntax\n    ---\n",
        "| Flag | Meaning |\n    |------|---------|\n| check | Verify docs |\n",
        "[build\n    +\ncache]: ./build-cache.md\n\n[build\n    +\ncache]\n",
    ] {
        assert_round_trip(input, &opts);
    }
}

#[test]
fn raw_text_inside_html_blocks_keeps_its_layout() {
    for input in [
        "<table class=\"examples\"><tr><td>\n<pre>\nmarkfmt --check .\n\n**verbatim flags**\n</pre>\n</td></tr></table>\n",
        "before <pre> text\n\nstill pre </pre> <code> literal\n\nmore code </code> after\n",
    ] {
        assert_round_trip(input, &options(8));
    }
}

#[test]
fn prompt_tags_after_prose_or_lists_stay_on_their_own_lines() {
    // The tags continue a paragraph or list item (CommonMark HTML block rule 7 cannot interrupt
    // a paragraph), so keeping them on their own lines doesn't change the structure.
    for input in [
        "Some intro text.\n<rules>\nDo the thing.\n</rules>\n",
        "<rules>\n\n- Do X\n- Do Y\n</rules>\n",
        "<example>\n\n1. First step\n2. Second step\n</example>\n",
    ] {
        assert_format(input, input, &options(72));
        assert_round_trip(input, &options(72));
    }
    // An indented tag inside a list item is item content and stays in the item.
    let nested = "- item\n  <rules>\n  text\n";
    assert_format(nested, nested, &options(72));
    assert_round_trip(nested, &options(72));

    // A column-0 prompt tag inside a list, heading, or paragraph must not strip container or
    // continuation context from lines that follow it, nor let a narrowed list absorb a following
    // indented HTML block across the tag.
    for mid_block in [
        "- item\n<rules>\n  ---\n</rules>\n",
        "- item\n<rules>\n  - nested\n</rules>\n",
        "- item\n<rules>\n  > quote\n</rules>\n",
        "Paragraph\n<rules>\n    indented\nfollowing\n</rules>\n",
        "-   item\n<rules>\n  <div>html</div>\n</rules>\n",
    ] {
        assert_round_trip(mid_block, &options(72));
    }
}

#[test]
fn reviewed_rendering_boundaries_round_trip() {
    for input in [
        "- [ ] \n  continued\n",
        "- [x] \n  Second.\n",
        "- [x] \n\n  Second.\n",
        "Before.\n\n- [x] \n\n  Second.\n\nAfter.\n",
        "> - [x] \n>\n>   Second.\n",
        "- [ ]\n\n      code\n",
        "- [ ] \nlazy\n",
        "- [x] \nlazy\n",
        "1. [ ] \nlazy\n",
        "> - [ ] \n> lazy\n",
        "- [ ] \n===\n",
        "- [ ]\ncontinued\n",
        "- [ ]\n===\n",
        "1. a\n1. [x]\ncontinued\n",
        "> [!NOTE]  \n> body\n",
        ">  [!TIP]  \n> body\n",
        "> [!NOTE] Title\n> Body\n",
        "R[^1].\n\n[^1]: a\\\n===\n",
        "R[^1].\n\n[^1]: a  \n===\n",
        "Ref[^a].\n\n[^a]: first  \n    second\nthird\n",
        "Foo.[^one two]\n\n[^one two]: Here is the footnote.\n",
        "hello[\u{a0}world](/u) and [word\u{a0}](/u)\n",
        "![   ](/img) and ![ a  b ](/img)\n",
        "![ a  b ][img]\n\n[img]: /img\n",
        "<pre/>\n\nSecond   paragraph   with    spaces.\n",
    ] {
        for wrap in [
            WrapMode::Always,
            WrapMode::Never,
            WrapMode::Preserve,
            WrapMode::Sentence,
        ] {
            for width in [4, 10, 12, 15, 30, 72] {
                let mut opts = options(width);
                opts.wrap = wrap;
                assert_round_trip(input, &opts);
            }
        }
    }
}

#[test]
fn leading_blank_quote_lines_do_not_promote_literal_alerts() {
    for (input, expected) in [
        (">\n> [!NOTE]\n", ">\n> [!NOTE]\n"),
        (">\n> [!NOTE]  \n> body\n", ">\n> [!NOTE]  \n> body\n"),
        (
            ">\n>\n> [!WARNING]\n> body\n",
            ">\n>\n> [!WARNING]\n> body\n",
        ),
        ("- >\n  > [!TIP]\n", "- >\n  > [!TIP]\n"),
        (">\n>     [!NOTE]\n> body\n", ">     [!NOTE]\n>\n> body\n"),
    ] {
        assert_format(input, expected, &options(72));
        for wrap in [
            WrapMode::Always,
            WrapMode::Never,
            WrapMode::Preserve,
            WrapMode::Sentence,
        ] {
            for width in [4, 8, 12, 30, 72] {
                let mut opts = options(width);
                opts.wrap = wrap;
                assert_round_trip(input, &opts);
            }
        }
    }
}

#[test]
fn paragraphs_after_definitions_and_rejected_definition_prefixes_still_wrap() {
    // Ordinary prose after a reference definition cannot attach as a link title and wraps normally.
    assert_format(
        "[id]: https://example.com\nThis is an ordinary paragraph following a link definition that wraps across lines.\n",
        "[id]: https://example.com\nThis is an ordinary paragraph\nfollowing a link definition that\nwraps across lines.\n",
        &options(36),
    );
    let mut sentence_opts = options(72);
    sentence_opts.wrap = WrapMode::Sentence;
    assert_format(
        "[id]: https://example.com\nFirst sentence here. Second sentence here.\n",
        "[id]: https://example.com\nFirst sentence here.\nSecond sentence here.\n",
        &sentence_opts,
    );

    // Rejected `[label]:` prefixes and title-like prefixes keep only their invalidating prefix
    // together on line 0, allowing the remainder of the paragraph to wrap.
    assert_format(
        "[Note]: This is a long note paragraph with enough words to wrap cleanly across lines.\n",
        "[Note]: This is a long note\nparagraph with enough words to wrap\ncleanly across lines.\n",
        &options(36),
    );
    assert_format(
        "[release]: /releases\n\"Release notes\" draft followed by additional words that wrap onto subsequent lines.\n",
        "[release]: /releases\n\"Release notes\" draft followed by\nadditional words that wrap onto\nsubsequent lines.\n",
        &options(36),
    );
    assert_format(
        "[release]: /releases \"Release notes\" draft followed by additional words that wrap onto subsequent lines.\n",
        "[release]: /releases \"Release notes\" draft\nfollowed by additional words that\nwrap onto subsequent lines.\n",
        &options(36),
    );

    for input in [
        "[id]: https://example.com\nThis is an ordinary paragraph following a link definition that wraps across lines.\n",
        "[Note]: This is a long note paragraph with enough words to wrap cleanly across lines.\n",
        "[release]: /releases\n\"Release notes\" draft followed by additional words that wrap.\n",
        "[release]: /releases \"Release notes\" draft followed by additional words that wrap.\n",
        "[release]: /releases\n\"Release  \nnotes\" draft followed by additional words that wrap.\n",
    ] {
        for wrap in [
            WrapMode::Always,
            WrapMode::Never,
            WrapMode::Preserve,
            WrapMode::Sentence,
        ] {
            for width in [6, 8, 20, 36, 72] {
                let mut opts = options(width);
                opts.wrap = wrap;
                assert_round_trip(input, &opts);
            }
        }
    }
}
