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

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use markfmt::formatter::format_file_content;
use markfmt::lint::lint_with_options;
use markfmt::range_filter::RangeFilter;
use markfmt::{format_markdown, FormattingOptions, OrderedListStyle, WrapMode};
use pretty_assertions::assert_eq;

mod common;
use common::{assert_format, assert_success, Project};

fn commit_opts(width: usize) -> FormattingOptions {
    let mut opts = FormattingOptions::default();
    opts.commit_message = true;
    opts.max_line_width = width;
    opts
}

#[test]
fn wrapping_commit_hazards_never_creates_mailinfo_patches() {
    let project = Project::new();
    let message_path = project.path().join("message");
    let patch_path = project.path().join("patch");
    for (separators, wrap) in [":", "-", "-:", "="]
        .into_iter()
        .flat_map(|s| [(s, WrapMode::Always), (s, WrapMode::Preserve)])
    {
        for width in [6, 10, 17] {
            for input in [
                "Subject\n\nalpha beta gamma Index: foo bar baz qux\n\nEnd.\n",
                "Subject\n\nStart :u alpha beta gamma delta\n\nEnd.\n",
                "Subject\n\nIndex:\tfoo\n\nEnd.\n",
                // Collapsing whitespace one line at a time (`--wrap preserve`) or in a
                // two-line setext heading.
                "Subject\n\nFirst line.\ndiff  -u a b\nMore text after.\n",
                "Subject\n\nFirst line.\nIndex:\tfoo bar\nMore text after.\n",
                "Subject\n\n diff -u a b\n============\n\nMore text after.\n",
                "Subject\n\ndiff  -u a b\n=============\n\nMore text after.\n",
                "Subject\n\n Index: foo\n============\n\nMore text after.\n",
            ] {
                let mut opts = commit_opts(width);
                opts.wrap = wrap;
                opts.trailer_separators = separators.to_string();
                common::assert_round_trip(input, &opts);
                let formatted = format_markdown(input, &opts);
                let mut messages = Vec::new();
                for text in [input, &formatted] {
                    git_stdout(
                        &[
                            "mailinfo",
                            message_path.to_str().unwrap(),
                            patch_path.to_str().unwrap(),
                        ],
                        text,
                    );
                    assert_eq!(fs::read(&patch_path).unwrap(), Vec::<u8>::new());
                    messages.push(fs::read_to_string(&message_path).unwrap());
                }
                assert_eq!(
                    messages[0].split_whitespace().collect::<Vec<_>>(),
                    messages[1].split_whitespace().collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn preserves_gerrit_change_id_multiline_relnote_and_long_trailers() {
    let input = concat!(
        "feat(parser): support nested admonitions in Markdeep definition lists and footnotes\n",
        "\n",
        "Previously, opening a nested admonition inside a definition list caused the outer container to terminate early when blank lines appeared.\n",
        "\n",
        "Relnote: \"Fixed nested admonitions inside definition lists so multi-paragraph\n",
        "notes keep their indentation.\"\n",
        "Bug: 123456789\n",
        "Test: ./gradlew :compose:foundation:foundation:connectedAndroidTest --info --rerun-tasks\n",
        "Co-authored-by: Very Long Contributor Name <very.long.contributor.email@corp.example.com>\n",
        "Reviewed-by: Reviewer <reviewer@example.com>\n",
        "  with RFC 822 folded continuation detail\n",
        "(cherry picked from commit 0123456789abcdef0123456789abcdef01234567)\n",
        "Merged-In: Iaabbccddeeff00112233445566778899aabbccdd\n",
        "Change-Id: I0123456789abcdef0123456789abcdef01234567\n",
    );
    let expected = concat!(
        "feat(parser): support nested admonitions in Markdeep definition lists and footnotes\n",
        "\n",
        "Previously, opening a nested admonition inside a definition list caused\n",
        "the outer container to terminate early when blank lines appeared.\n",
        "\n",
        "Relnote: \"Fixed nested admonitions inside definition lists so multi-paragraph\n",
        "notes keep their indentation.\"\n",
        "Bug: 123456789\n",
        "Test: ./gradlew :compose:foundation:foundation:connectedAndroidTest --info --rerun-tasks\n",
        "Co-authored-by: Very Long Contributor Name <very.long.contributor.email@corp.example.com>\n",
        "Reviewed-by: Reviewer <reviewer@example.com>\n",
        "  with RFC 822 folded continuation detail\n",
        "(cherry picked from commit 0123456789abcdef0123456789abcdef01234567)\n",
        "Merged-In: Iaabbccddeeff00112233445566778899aabbccdd\n",
        "Change-Id: I0123456789abcdef0123456789abcdef01234567\n",
    );
    assert_format(input, expected, &commit_opts(72));
    assert!(lint_with_options(input, &commit_opts(72)).is_empty());
}

#[test]
fn preserves_git_comments_tabs_and_verbose_scissors_diff_without_spurious_lints() {
    let input = concat!(
        "Fix table pipe handling in parser\n",
        "\n",
        "Reflows the body explanation cleanly to seventy-two columns while leaving the git status comments and verbose diff completely untouched.\n",
        "\n",
        "Change-Id: I0123456789abcdef0123456789abcdef01234567\n",
        "# Please enter the commit message for your changes. Lines starting\n",
        "# with '#' will be ignored, and an empty message aborts the commit.\n",
        "#\n",
        "# On branch main\n",
        "# Changes to be committed:\n",
        "#\tmodified:   src/table.rs\n",
        "#\n",
        "# ------------------------ >8 ------------------------\n",
        "# Do not modify or remove the line above.\n",
        "# Everything below it will be ignored.\n",
        "diff --git a/tests/table.md b/tests/table.md\n",
        "--- a/tests/table.md\n",
        "+++ b/tests/table.md\n",
        "@@ -1,3 +1,3 @@\n",
        " | a | b |\n",
        " | - | - |\n",
        "+| `un|escaped` | 2 |\n",
    );
    let expected = concat!(
        "Fix table pipe handling in parser\n",
        "\n",
        "Reflows the body explanation cleanly to seventy-two columns while\n",
        "leaving the git status comments and verbose diff completely untouched.\n",
        "\n",
        "Change-Id: I0123456789abcdef0123456789abcdef01234567\n",
        "# Please enter the commit message for your changes. Lines starting\n",
        "# with '#' will be ignored, and an empty message aborts the commit.\n",
        "#\n",
        "# On branch main\n",
        "# Changes to be committed:\n",
        "#\tmodified:   src/table.rs\n",
        "#\n",
        "# ------------------------ >8 ------------------------\n",
        "# Do not modify or remove the line above.\n",
        "# Everything below it will be ignored.\n",
        "diff --git a/tests/table.md b/tests/table.md\n",
        "--- a/tests/table.md\n",
        "+++ b/tests/table.md\n",
        "@@ -1,3 +1,3 @@\n",
        " | a | b |\n",
        " | - | - |\n",
        "+| `un|escaped` | 2 |\n",
    );
    let opts = commit_opts(72);
    assert_format(input, expected, &opts);
    assert!(
        lint_with_options(input, &opts).is_empty(),
        "diff below scissors must not trigger table pipe warnings"
    );

    // `8<` is not Git's commit cut line and must not prematurely stop formatting.
    let fake_scissors = concat!(
        "Subject\n",
        "\n",
        "# ------------------------ 8< ------------------------\n",
        "\n",
        "This   paragraph   after   a   non-Git   perforation   still   formats.\n",
    );
    assert_format(
        fake_scissors,
        concat!(
            "Subject\n",
            "\n",
            "# ------------------------ 8< ------------------------\n",
            "\n",
            "This paragraph after a non-Git\n",
            "perforation still formats.\n",
        ),
        &commit_opts(35),
    );
}

#[test]
fn warns_on_unseparated_trailer_while_keeping_it_on_its_own_line() {
    let opts = commit_opts(50);

    // Attached to prose without a blank line: kept on its own line, and linted.
    let prose_attached = concat!(
        "Fix race condition\n",
        "\n",
        "Ensure worker threads join before closing the channel in shutdown.\n",
        "Change-Id: I0123456789abcdef0123456789abcdef01234567\n",
    );
    assert_format(
        prose_attached,
        concat!(
            "Fix race condition\n",
            "\n",
            "Ensure worker threads join before closing the\n",
            "channel in shutdown.\n",
            "Change-Id: I0123456789abcdef0123456789abcdef01234567\n",
        ),
        &opts,
    );
    let warnings = lint_with_options(prose_attached, &opts);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].line, 4);
    assert!(
        warnings[0]
            .message
            .contains("`Change-Id:` is not separated"),
        "{:?}",
        warnings[0]
    );

    // Attached directly under subject line without a blank line: also linted.
    let subject_attached = "Fix typo\nChange-Id: I0123456789abcdef0123456789abcdef01234567\n";
    let warnings = lint_with_options(subject_attached, &opts);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].line, 2);
}

#[test]
fn avoids_hash_stripspace_and_git_am_patchbreak_hazards_at_column_zero() {
    let opts = commit_opts(36);
    let input = concat!(
        "Guard commit message hazards\n",
        "\n",
        "Resolves issue #123 when callers run diff -u or check Index: headers in patches.\n",
    );
    let formatted = format_markdown(input, &opts);
    for line in formatted.lines().skip(2) {
        assert!(
            !line.starts_with('#'),
            "hash wrapped to col 0: {formatted:?}"
        );
        assert!(
            !line.starts_with("diff -"),
            "diff - wrapped to col 0: {formatted:?}"
        );
        assert!(
            !line.starts_with("Index:"),
            "Index: wrapped to col 0: {formatted:?}"
        );
    }

    // Verify `git mailinfo` preserves the entire body without truncating at `diff -u` or `Index:`.
    let tmp = tempfile::TempDir::new().unwrap();
    let msg_out = tmp.path().join("msg");
    let patch_out = tmp.path().join("patch");
    let email = format!("From: A <a@b.c>\nSubject: [PATCH] x\n\n{formatted}");
    let mut child = Command::new("git")
        .arg("mailinfo")
        .arg(&msg_out)
        .arg(&patch_out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(email.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    let extracted = fs::read_to_string(&msg_out).unwrap();
    assert!(
        extracted.contains("in patches."),
        "git mailinfo truncated commit message: {extracted:?}"
    );
}

#[test]
fn comment_lines_keep_list_nesting_and_code_spans() {
    let opts = commit_opts(30);
    // Git strips `#comment` and rejoins the child to its parent item, so the child must keep its
    // indentation (tight or after blank lines).
    assert_format(
        "Subject\n\n- parent\n#comment\n  - child\n",
        "Subject\n\n- parent\n#comment\n  - child\n",
        &opts,
    );
    assert_format(
        "Subject\n\n- parent\n\n#comment\n\n  continuation   of the parent\n",
        "Subject\n\n- parent\n\n#comment\n\n  continuation   of the parent\n",
        &opts,
    );
    // A `#` line closing a code span is literal text for CommonMark but a whole comment line for
    // Git: neither the span's spaces nor that line may change.
    assert_format(
        "Subject\n\nfoo `one   two\n#end` tail words\n",
        "Subject\n\nfoo `one   two\n#end` tail words\n",
        &opts,
    );
}

#[test]
fn trailers_above_patch_divider_and_custom_separators_stay_intact() {
    // Git looks for trailers only above `---`; a setext underline there must not be widened.
    let divider = "Subject\n\nBody text.\n\nCustom: a long trailer value that is wider than thirty\n---\nnotes\n";
    assert_format(divider, divider, &commit_opts(30));
    let prose = "Subject\n\nSome prose here\n---\nmore\n";
    assert_format(prose, prose, &commit_opts(72));

    // With `trailer.separators = :=`, `Key=value` lines are trailers too.
    let mut opts = commit_opts(30);
    opts.trailer_separators = ":=".to_string();
    let equals = "Subject\n\nBody.\n\nReviewed-by=Some Person With A Long Name <p@e.x>\n";
    assert_format(equals, equals, &opts);
    assert_format(
        "Subject\n\nThe words a=b and x=1 wrap freely here.\n",
        "Subject\n\nThe words a=b and x=1 wrap\nfreely here.\n",
        &opts,
    );
}

#[test]
fn scissors_line_after_prefix_ending_in_space_detaches_diff() {
    // Git writes the configured `// ` and then its own space before the marker.
    let mut opts = commit_opts(72);
    opts.comment_char = "// ".to_string();
    let input = concat!(
        "Subject\n",
        "\n",
        "Body.\n",
        "\n",
        "//  ------------------------ >8 ------------------------\n",
        "// Do not modify or remove the line above.\n",
        "diff --git a/x b/x\n",
        "+a  b   c\n",
    );
    assert_format(input, input, &opts);
}

#[test]
fn keeps_comment_lines_after_hard_breaks_and_indented_hazards_off_column_zero() {
    // A comment line after a hard break stays a paragraph line for CommonMark, but Git strips it:
    // the paragraph is kept as written so no following text is joined onto it. An indented `#123` is not a comment, and must not
    // become one when the paragraph's indentation is dropped.
    let input = concat!(
        "Subject\n",
        "\n",
        "  #123 is fixed by this change\n",
        "\n",
        "Line one  \n",
        "#foo is a comment line that is far longer than the configured width\n",
        "more text\n",
        "here\n",
        "\n",
        "A\\\n",
        "   diff -u here\n",
        "\n",
        "  plain indented prose\n",
    );
    let expected = concat!(
        "Subject\n",
        "\n",
        " #123 is fixed by this change\n",
        "\n",
        "Line one  \n",
        "#foo is a comment line that is far longer than the configured width\n",
        "more text\n",
        "here\n",
        "\n",
        "A\\\n",
        " diff -u here\n",
        "\n",
        "plain indented prose\n",
    );
    assert_format(input, expected, &commit_opts(72));

    let narrow = format_markdown(input, &commit_opts(20));
    let comment = "#foo is a comment line that is far longer than the configured width";
    for line in narrow.lines() {
        assert!(
            line == comment || !line.starts_with(['#', 'd']),
            "hazard at column 0: {narrow:?}"
        );
    }
    assert!(narrow.lines().any(|line| line == comment), "{narrow:?}");
}

#[test]
fn keeps_markdown_shaped_and_autosquash_subjects_verbatim() {
    let opts = commit_opts(40);
    for subject in [
        "fixup! Add optimal Knuth-Plass paragraph balancing for longer Markdown text",
        "squash!   Keep   double   spaces   for   autosquash   matching",
        "*   bump   dependencies   without   normalizing   bullet   subject",
        "1.   Keep   numbered   subject   line   verbatim",
        ">   Keep   quoted   subject   line   verbatim",
    ] {
        let input =
            format!("{subject}\n\nBody   text   that   wraps   at   forty   columns   cleanly.\n");
        let expected = format!("{subject}\n\nBody text that wraps at forty columns\ncleanly.\n");
        assert_format(&input, &expected, &opts);
    }

    // A multi-line opening paragraph (e.g. a body selection piped via `gq` in Vim) still reflows.
    let selection =
        "This   selected   body   paragraph   spans   two\nunformatted   lines   in   the   editor.\n";
    assert_format(
        selection,
        "This selected body paragraph spans two\nunformatted lines in the editor.\n",
        &opts,
    );
}

#[test]
fn preserves_empty_commit_squash_trailers_jujutsu_and_pgp_signatures() {
    let opts = commit_opts(50);

    // Empty commit template keeps its leading blank cursor line so Git aborts empty commits.
    let empty_commit = "\n# Please enter the commit message for your changes.\n# On branch main\n";
    assert_format(empty_commit, empty_commit, &opts);

    // Squash buffer with mid-file trailer block + mid-file `Note:` prose paragraph.
    let squash = concat!(
        "Squashed commit summary\n",
        "\n",
        "First commit explanation that is long enough to wrap at fifty columns.\n",
        "\n",
        "Bug: 111\n",
        "Change-Id: I1111111111111111111111111111111111111111\n",
        "\n",
        "Note: This mid-body note is ordinary prose and therefore wraps cleanly at fifty columns.\n",
        "\n",
        "Bug: 222\n",
        "Change-Id: I2222222222222222222222222222222222222222\n",
    );
    let expected_squash = concat!(
        "Squashed commit summary\n",
        "\n",
        "First commit explanation that is long enough to\n",
        "wrap at fifty columns.\n",
        "\n",
        "Bug: 111\n",
        "Change-Id: I1111111111111111111111111111111111111111\n",
        "\n",
        "Note: This mid-body note is ordinary prose and\n",
        "therefore wraps cleanly at fifty columns.\n",
        "\n",
        "Bug: 222\n",
        "Change-Id: I2222222222222222222222222222222222222222\n",
    );
    assert_format(squash, expected_squash, &opts);

    // Jujutsu description buffer with blank line inside trailing JJ: block + JJ: ignore-rest.
    let jj = concat!(
        "Refactor discovery\n",
        "\n",
        "Body   prose   that   wraps   cleanly   to   fifty   columns   here.\n",
        "\n",
        "JJ: This commit contains the following changes:\n",
        "JJ:     M src/discovery.rs\n",
        "\n",
        "JJ: Lines starting with \"JJ:\" (like this one) will be removed.\n",
        "JJ: ignore-rest\n",
        "diff --git a/src/discovery.rs b/src/discovery.rs\n",
    );
    let expected_jj = concat!(
        "Refactor discovery\n",
        "\n",
        "Body prose that wraps cleanly to fifty columns\n",
        "here.\n",
        "\n",
        "JJ: This commit contains the following changes:\n",
        "JJ:     M src/discovery.rs\n",
        "\n",
        "JJ: Lines starting with \"JJ:\" (like this one) will be removed.\n",
        "JJ: ignore-rest\n",
        "diff --git a/src/discovery.rs b/src/discovery.rs\n",
    );
    assert_format(jj, expected_jj, &opts);

    // PGP signature in TAG_EDITMSG stays untouched.
    let signed_tag = concat!(
        "Release v1.2.0\n",
        "\n",
        "Summary   of   highlights   in   this   release   spanning   fifty   columns.\n",
        "\n",
        "-----BEGIN PGP SIGNATURE-----\n",
        "\n",
        "iQIzBAABCAAdFiEE0123456789abcdef==\n",
        "-----END PGP SIGNATURE-----\n",
    );
    let expected_tag = concat!(
        "Release v1.2.0\n",
        "\n",
        "Summary of highlights in this release spanning\n",
        "fifty columns.\n",
        "\n",
        "-----BEGIN PGP SIGNATURE-----\n",
        "\n",
        "iQIzBAABCAAdFiEE0123456789abcdef==\n",
        "-----END PGP SIGNATURE-----\n",
    );
    assert_format(signed_tag, expected_tag, &opts);
}

#[test]
fn cli_auto_detection_config_isolation_comment_char_and_git_amend() {
    let project = Project::new();
    project.init_git();

    // Top-level `.markfmt.toml` has doc-only width 100 and excludes `*`, while `[commit_message]`
    // sets width 45.
    project.write(
        ".markfmt.toml",
        concat!(
            "max_line_width = 100\n",
            "exclude = [\"*\"]\n",
            "\n",
            "[commit_message]\n",
            "max_line_width = 45\n",
        ),
    );
    project.write("README.md", "# Doc\n");
    project.commit();

    // Auto-detects `.jjdescription` and uses `[commit_message]` width 45 (not top-level 100),
    // ignoring `.markfmt.toml` `exclude = ["*"]`, with silent stdout.
    project.write(
        "change.jjdescription",
        "Subject\n\nBody paragraph that wraps at forty-five columns from commit_message config.\n",
    );
    let jj_out = project.run(&["change.jjdescription"]);
    assert_success(&jj_out);
    assert_eq!(String::from_utf8_lossy(&jj_out.stdout), "");
    assert_eq!(
        project.read("change.jjdescription"),
        "Subject\n\nBody paragraph that wraps at forty-five\ncolumns from commit_message config.\n"
    );

    // Custom `core.commentChar = ";"` in Git config allows Markdown `# Heading` in commit body
    // while preserving `;` comments and gluing `;123` away from column 0!
    project.git(&["config", "core.commentChar", ";"]);
    let semi_msg = concat!(
        "Support custom commentChar\n",
        "\n",
        "# Details\n",
        "\n",
        "Avoids wrapping before ;123 token at forty-five columns.\n",
        "\n",
        "Change-Id: I0123456789abcdef0123456789abcdef01234567\n",
        "; Please enter the commit message\n",
        ";\tmodified:   README.md\n",
    );
    project.write(".git/COMMIT_EDITMSG", semi_msg);
    assert_success(&project.run(&[".git/COMMIT_EDITMSG"]));
    assert_eq!(
        project.read(".git/COMMIT_EDITMSG"),
        concat!(
            "Support custom commentChar\n",
            "\n",
            "# Details\n",
            "\n",
            "Avoids wrapping before ;123 token at\n",
            "forty-five columns.\n",
            "\n",
            "Change-Id: I0123456789abcdef0123456789abcdef01234567\n",
            "; Please enter the commit message\n",
            ";\tmodified:   README.md\n",
        )
    );

    // `core.commentChar = auto` resolves `;` when `#` is used in the message body.
    project.git(&["config", "core.commentChar", "auto"]);
    let auto_msg = concat!(
        "Auto commentChar\n",
        "\n",
        "# Section\n",
        "\n",
        "Body   with   extra   spaces   that   wraps   at   forty-five   columns.\n",
        "; Please enter the commit message for your changes. Lines starting\n",
        "; with ';' will be ignored, and an empty message aborts the commit.\n",
    );
    project.write(".git/COMMIT_EDITMSG", auto_msg);
    assert_success(&project.run(&[".git/COMMIT_EDITMSG"]));
    assert_eq!(
        project.read(".git/COMMIT_EDITMSG"),
        concat!(
            "Auto commentChar\n",
            "\n",
            "# Section\n",
            "\n",
            "Body with extra spaces that wraps at\n",
            "forty-five columns.\n",
            "; Please enter the commit message for your changes. Lines starting\n",
            "; with ';' will be ignored, and an empty message aborts the commit.\n",
        )
    );

    // `--wrap preserve` keeps pasted stack trace line breaks while normalizing spacing & trailers.
    let stack_trace = concat!(
        "Fix NPE in analyzer\n",
        "\n",
        "java.lang.NullPointerException:   boom\n",
        "  at com.example.Lint.run(Lint.java:42)\n",
        "  at com.example.Main.main(Main.java:10)\n",
        "\n",
        "Bug: 98765\n",
        "Change-Id: I0123456789abcdef0123456789abcdef01234567\n",
    );
    let preserved = project.stdin(&["--commit-msg", "--wrap", "preserve", "-"], stack_trace);
    assert_success(&preserved);
    assert_eq!(
        String::from_utf8_lossy(&preserved.stdout),
        concat!(
            "Fix NPE in analyzer\n",
            "\n",
            "java.lang.NullPointerException: boom\n",
            "  at com.example.Lint.run(Lint.java:42)\n",
            "  at com.example.Main.main(Main.java:10)\n",
            "\n",
            "Bug: 98765\n",
            "Change-Id: I0123456789abcdef0123456789abcdef01234567\n",
        )
    );

    // Real `GIT_EDITOR=markfmt git commit --amend` end-to-end test!
    project.git(&["config", "--unset", "core.commentChar"]);
    project.git(&[
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--allow-empty",
        "-qm",
        "Amend target\n\nUnformatted   paragraph   that   exceeds   forty-five   columns   during   amend.\n\nChange-Id: I0123456789abcdef0123456789abcdef01234567",
    ]);
    let amend = Command::new("git")
        .current_dir(project.path())
        // Git runs the editor through `sh`, so quote the path to keep Windows backslashes.
        .env(
            "GIT_EDITOR",
            format!("\"{}\"", env!("CARGO_BIN_EXE_markfmt")),
        )
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--amend",
            "--allow-empty",
        ])
        .output()
        .unwrap();
    assert_success(&amend);
    let log = Command::new("git")
        .current_dir(project.path())
        .args(["log", "-1", "--format=%B"])
        .output()
        .unwrap();
    assert_success(&log);
    assert_eq!(
        String::from_utf8_lossy(&log.stdout).replace("\r\n", "\n"),
        concat!(
            "Amend target\n",
            "\n",
            "Unformatted paragraph that exceeds forty-five\n",
            "columns during amend.\n",
            "\n",
            "Change-Id: I0123456789abcdef0123456789abcdef01234567\n\n",
        )
    );
}

#[test]
fn commit_editor_keeps_successful_edits_when_formatting_fails() {
    let project = Project::new();
    project.init_git();
    let bin = env!("CARGO_BIN_EXE_markfmt");
    let edited = "Edited subject\n\nBody   with   intentional   spacing.\n";
    let editor = "printf 'Edited subject\\n\\nBody   with   intentional   spacing.\\n' >";
    for bad_config in [true, false] {
        if bad_config {
            project.write(".markfmt.toml", "not valid toml");
        } else {
            project.write(".markfmt.toml", "");
            project.git(&["config", "trailer.separators", "a"]);
        }
        let out = Command::new("git")
            .current_dir(project.path())
            .env("GIT_EDITOR", format!("\"{bin}\" --commit-editor"))
            .env("MARKFMT_EDITOR", editor)
            .args(["-c", "commit.gpgsign=false", "commit", "--allow-empty"])
            .output()
            .unwrap();
        assert_success(&out);
        assert!(String::from_utf8_lossy(&out.stderr).contains("left as edited"));
        assert_eq!(project.read(".git/COMMIT_EDITMSG"), edited);
        let log = Command::new("git")
            .current_dir(project.path())
            .args(["log", "-1", "--format=%B"])
            .output()
            .unwrap();
        assert_success(&log);
        assert_eq!(String::from_utf8_lossy(&log.stdout), format!("{edited}\n"));

        // The same failure outside the editor wrapper remains fatal and leaves the file alone.
        assert_eq!(project.run(&[".git/COMMIT_EDITMSG"]).status.code(), Some(2));
        assert_eq!(project.read(".git/COMMIT_EDITMSG"), edited);
    }

    // Read failures after editing are also non-fatal; the bytes the editor wrote survive.
    let out = project.run(&[
        "--no-config",
        "--trailer-separators=:",
        "--commit-editor=printf '\\377' >",
        ".git/COMMIT_EDITMSG",
    ]);
    assert_success(&out);
    assert!(String::from_utf8_lossy(&out.stderr).contains("left as edited"));
    assert_eq!(
        fs::read(project.path().join(".git/COMMIT_EDITMSG")).unwrap(),
        [255]
    );
}

#[cfg(debug_assertions)]
#[test]
fn formatter_panics_fail_one_file_and_never_abort_a_commit() {
    let project = Project::new();
    project.init_git();
    let bin = env!("CARGO_BIN_EXE_markfmt");
    let edited = "Edited subject\n\nBody   with   intentional   spacing.\n";
    let editor = "printf 'Edited subject\\n\\nBody   with   intentional   spacing.\\n' >";

    // Under `--commit-editor`, a panic leaves the message as edited and the commit goes ahead.
    let out = Command::new("git")
        .current_dir(project.path())
        .env("GIT_EDITOR", format!("\"{bin}\" --commit-editor"))
        .env("MARKFMT_EDITOR", editor)
        .env("MARKFMT_TEST_PANIC", "1")
        .args(["-c", "commit.gpgsign=false", "commit", "--allow-empty"])
        .output()
        .unwrap();
    assert_success(&out);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("left as edited"), "{stderr}");
    assert!(stderr.contains("Internal error formatting"), "{stderr}");
    assert_eq!(project.read(".git/COMMIT_EDITMSG"), edited);

    // Elsewhere it is a failure (exit 2) that leaves the file unchanged.
    let out = Command::new(bin)
        .current_dir(project.path())
        .env("MARKFMT_TEST_PANIC", "1")
        .arg(".git/COMMIT_EDITMSG")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Internal error formatting"));
    assert_eq!(project.read(".git/COMMIT_EDITMSG"), edited);
}

#[test]
fn commit_editor_launches_editor_formats_allowlist_and_guards_recursion() {
    let project = Project::new();
    project.init_git();
    project.write(".markfmt.toml", "[commit_message]\nmax_line_width = 45\n");
    project.write("README.md", "# Test\n");
    project.commit();

    let bin = env!("CARGO_BIN_EXE_markfmt");

    // 1. End-to-end `core.editor = "<markfmt> --commit-editor"` with `$VISUAL` containing shell
    // arguments (`printf ... > "$1"`), ignoring `$GIT_EDITOR` self-reference!
    let core_editor = format!("\"{bin}\" --commit-editor");
    project.git(&["config", "core.editor", &core_editor]);

    let fake_visual = "printf 'Edited subject\\n\\nBody   written   interactively   that   exceeds   forty-five   columns.\\n\\nBug: 777\\nChange-Id: I0123456789abcdef0123456789abcdef01234567\\n' >";
    let commit_out = Command::new("git")
        .current_dir(project.path())
        .env_remove("GIT_EDITOR")
        .env("VISUAL", fake_visual)
        .args(["-c", "commit.gpgsign=false", "commit", "--allow-empty"])
        .output()
        .unwrap();
    assert_success(&commit_out);

    let log = Command::new("git")
        .current_dir(project.path())
        .args(["log", "-1", "--format=%B"])
        .output()
        .unwrap();
    assert_success(&log);
    assert_eq!(
        String::from_utf8_lossy(&log.stdout).replace("\r\n", "\n"),
        concat!(
            "Edited subject\n",
            "\n",
            "Body written interactively that exceeds\n",
            "forty-five columns.\n",
            "\n",
            "Bug: 777\n",
            "Change-Id: I0123456789abcdef0123456789abcdef01234567\n\n",
        )
    );

    // 2. Non-commit files (`git-rebase-todo`, `.gitsendemail.msg.0001`, `ADD_EDIT.patch`) are
    // edited by the underlying editor and NOT reformatted (allowlist `is_commit_message_path`).
    for non_commit in [
        ".git/rebase-merge/git-rebase-todo",
        ".gitsendemail.msg.0001",
        ".git/ADD_EDIT.patch",
        ".gitconfig",
    ] {
        project.write(
            non_commit,
            "pick 1111111   initial\nsquash 2222222   second\n",
        );
        let out = project
            .command()
            .env(
                "VISUAL",
                "printf 'pick 1111111   edited\\nsquash 2222222   kept\\n' >",
            )
            .args(["--commit-editor", non_commit])
            .output()
            .unwrap();
        assert_success(&out);
        assert_eq!(
            project.read(non_commit),
            "pick 1111111   edited\nsquash 2222222   kept\n",
            "non-commit file `{non_commit}` must not be reflowed"
        );
    }

    // 3. Explicit `--commit-editor=<CMD>` overrides `$MARKFMT_EDITOR` / `$VISUAL` / `$EDITOR` and
    // works reliably with file paths containing spaces!
    let spaced_dir = project.path().join("dir with spaces");
    fs::create_dir_all(&spaced_dir).unwrap();
    let spaced_msg = spaced_dir.join("COMMIT_EDITMSG");
    fs::write(&spaced_msg, "Old\n").unwrap();
    let explicit_out = project
        .command()
        .env("MARKFMT_EDITOR", "false")
        .env("VISUAL", "false")
        .arg("--commit-editor=printf 'Spaced path\\n\\nOne   two   three   four   five   six   seven   eight   nine   ten.\\n' >")
        .arg(&spaced_msg)
        .output()
        .unwrap();
    assert_success(&explicit_out);
    assert_eq!(
        fs::read_to_string(&spaced_msg).unwrap(),
        "Spaced path\n\nOne two three four five six seven eight nine\nten.\n"
    );

    // 4. Non-zero exit code from the underlying editor (e.g. `:cq` in Vim) propagates directly
    // without modifying the file!
    let abort_msg =
        "Subject\n\nUnformatted   body   that   must   remain   untouched   on   abort.\n";
    project.write(".git/COMMIT_EDITMSG", abort_msg);
    let aborted = project
        .command()
        .args(["--commit-editor=sh -c 'exit 3'", ".git/COMMIT_EDITMSG"])
        .output()
        .unwrap();
    assert_eq!(aborted.status.code(), Some(3));
    assert_eq!(project.read(".git/COMMIT_EDITMSG"), abort_msg);

    // 5. Direct and indirect recursion guards exit 2 cleanly instead of fork-bombing!
    let direct_rec = project
        .command()
        .arg(format!("--commit-editor=\"{bin}\" --commit-editor"))
        .arg(".git/COMMIT_EDITMSG")
        .output()
        .unwrap();
    assert_eq!(direct_rec.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&direct_rec.stderr).contains("Recursive `--commit-editor`"),
        "{}",
        String::from_utf8_lossy(&direct_rec.stderr)
    );

    // A wrapper script hides `markfmt` from the command-line check; the guard still sees that a
    // `--commit-editor` process is its ancestor.
    let indirect_rec = project
        .command()
        .env("MARKFMT_TEST_BIN", bin)
        .arg("--commit-editor=sh -c '\"$MARKFMT_TEST_BIN\" --commit-editor=true \"$@\"' sh")
        .arg(".git/COMMIT_EDITMSG")
        .output()
        .unwrap();
    assert_eq!(indirect_rec.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&indirect_rec.stderr).contains("Recursive `--commit-editor`"),
        "{}",
        String::from_utf8_lossy(&indirect_rec.stderr)
    );

    // A commit started from inside the editor (another repository's message) is not recursion,
    // and is formatted as usual.
    project.write("other/.git/COMMIT_EDITMSG", "Nested\n\nBody   text.\n");
    let nested = project
        .command()
        .env("MARKFMT_TEST_BIN", bin)
        .arg(
            "--commit-editor=sh -c '\"$MARKFMT_TEST_BIN\" --commit-editor=true other/.git/COMMIT_EDITMSG' sh",
        )
        .arg(".git/COMMIT_EDITMSG")
        .output()
        .unwrap();
    assert_success(&nested);
    assert_eq!(
        project.read("other/.git/COMMIT_EDITMSG"),
        "Nested\n\nBody text.\n"
    );

    // A long-lived editor started by an earlier commit passes `MARKFMT_IN_COMMIT_EDITOR`
    // (`<pid>:<tempfile>`) on to its terminals; once that `markfmt` has exited and removed its
    // marker file, a new commit from such a terminal is not recursion on any platform.
    let gone_marker = project.path().join("deleted-editor-marker");
    let leaked_marker = project
        .command()
        .env(
            "MARKFMT_IN_COMMIT_EDITOR",
            format!("{}:{}", std::process::id(), gone_marker.display()),
        )
        .args(["--commit-editor=true", ".git/COMMIT_EDITMSG"])
        .output()
        .unwrap();
    assert_success(&leaked_marker);

    // If a killed launcher left its marker behind, Unix also walks the process tree when allowed.
    #[cfg(unix)]
    {
        let ppid_accessible = std::fs::metadata("/proc/self/stat").is_ok()
            || Command::new("ps")
                .args(["-o", "ppid=", "-p", &std::process::id().to_string()])
                .output()
                .is_ok_and(|o| o.status.success());
        if ppid_accessible {
            let mut finished = Command::new("true").spawn().unwrap();
            let finished_pid = finished.id();
            finished.wait().unwrap();
            let leaked = project
                .command()
                .env("MARKFMT_IN_COMMIT_EDITOR", finished_pid.to_string())
                .args(["--commit-editor=true", ".git/COMMIT_EDITMSG"])
                .output()
                .unwrap();
            assert_success(&leaked);
        }
    }
}

#[test]
fn trailer_separators_come_from_git_config_cli_and_markfmt_toml() {
    let project = Project::new();
    project.init_git();
    project.git(&["config", "trailer.separators", ":="]);
    let msg = "Subject\n\nBody.\n\nReviewed-by=Some Person With A Long Name <p@e.x>\n";

    // Detected from `git config trailer.separators`.
    project.write(".git/COMMIT_EDITMSG", msg);
    assert_success(&project.run(&["--max-line-width=30", ".git/COMMIT_EDITMSG"]));
    assert_eq!(project.read(".git/COMMIT_EDITMSG"), msg);

    // `--trailer-separators` overrides Git: with only `:`, the line is prose and wraps.
    assert_success(&project.run(&[
        "--max-line-width=30",
        "--trailer-separators=:",
        ".git/COMMIT_EDITMSG",
    ]));
    assert_ne!(project.read(".git/COMMIT_EDITMSG"), msg);

    // `[commit_message] trailer_separators` is validated.
    project.write(
        ".markfmt.toml",
        "[commit_message]\ntrailer_separators = \"a\"\n",
    );
    let invalid = project.run(&[".git/COMMIT_EDITMSG"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&invalid.stderr).contains("trailer_separators"),
        "{}",
        String::from_utf8_lossy(&invalid.stderr)
    );
}

#[test]
fn hyphen_trailers_are_preserved_and_unsupported_git_separators_fail_safely() {
    let project = Project::new();
    project.init_git();
    project.git(&["config", "trailer.separators", "-"]);
    let msg = "Subject\n\nReview- Alice Bob Charlie Delta Echo Foxtrot Golf Hotel India Juliet\n";
    let parse = |text: &str| {
        git_stdout(
            &[
                "-c",
                "trailer.separators=-",
                "interpret-trailers",
                "--parse",
            ],
            text,
        )
    };
    assert!(!parse(msg).is_empty());
    project.write(".git/COMMIT_EDITMSG", msg);
    for _ in 0..2 {
        assert_success(&project.run(&["--max-line-width=24", ".git/COMMIT_EDITMSG"]));
        let formatted = project.read(".git/COMMIT_EDITMSG");
        assert_eq!(formatted, msg);
        assert_eq!(parse(&formatted), parse(msg));
    }

    // Git accepts separators outside markfmt's supported set. Never silently use `:` instead.
    project.git(&["config", "trailer.separators", "a"]);
    let rejected = project.run(&["--max-line-width=24", ".git/COMMIT_EDITMSG"]);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("Git trailer.separators"));
    assert_eq!(project.read(".git/COMMIT_EDITMSG"), msg);

    // An explicit override takes precedence over an unsupported Git setting.
    assert_success(&project.run(&["--trailer-separators=-", ".git/COMMIT_EDITMSG"]));
    assert_eq!(project.read(".git/COMMIT_EDITMSG"), msg);
}

#[test]
fn divider_notes_frozen_indent_and_literal_dividers_survive() {
    // Keeping a URL paragraph from turning into a trailer block must not drop what follows the
    // `---` divider.
    let notes =
        "Subject\n\nhttps://example.com\nwords on line two\n\n---\nnotes after the divider\n";
    assert_format(notes, notes, &commit_opts(72));

    // A frozen paragraph (a `#` line inside a code span) keeps its indentation, so ` #123`
    // doesn't become a comment line.
    let frozen = "Subject\n\n #123 code `one   two\n#end` tail\n";
    assert_format(frozen, frozen, &commit_opts(72));

    // A `---` line inside a code span is literal text, not a divider to split at.
    assert_format(
        "Subject\n\nfoo `one   two\n--- content` tail\n",
        "Subject\n\nfoo\n`one   two\n--- content`\ntail\n",
        &commit_opts(12),
    );
}

#[test]
fn configured_trailer_keys_are_recognized_and_unicode_separators_are_safe() {
    // With `trailer.review.key = Reviewed-by`, Git accepts this mixed paragraph as a trailer
    // block (one recognized trailer for up to three other lines), so it must not be reflowed.
    let mut opts = commit_opts(25);
    opts.trailer_keys = vec!["review".to_string(), "Reviewed-by".to_string()];
    let mixed = "Subject\n\nBody.\n\nThis is one long prose line that will wrap into several lines at a narrow width\nReviewed-by: Some Reviewer\n";
    assert_format(mixed, mixed, &opts);

    // Library callers can set any separator; a multi-byte one must not panic.
    let mut opts = commit_opts(72);
    opts.trailer_separators = "§".to_string();
    format_markdown(
        "Subject\n\nReviewed-by§ Reviewer Name\n\nMore text.\n",
        &opts,
    );

    let project = Project::new();
    project.init_git();
    project.git(&["config", "trailer.review.key", "Reviewed-by"]);
    project.write(".git/COMMIT_EDITMSG", mixed);
    assert_success(&project.run(&["--max-line-width=25", ".git/COMMIT_EDITMSG"]));
    assert_eq!(project.read(".git/COMMIT_EDITMSG"), mixed);

    let rejected = project.run(&["--trailer-separators=§", ".git/COMMIT_EDITMSG"]);
    assert_eq!(rejected.status.code(), Some(2));
}

#[test]
fn only_the_last_configured_trailer_key_is_recognized() {
    // Git keeps the last `trailer.<name>.key` (warning about the others), so `OldKey:` is not a
    // recognized trailer and the mixed paragraph is prose that can be wrapped.
    let project = Project::new();
    project.init_git();
    project.git(&["config", "--add", "trailer.foo.key", "OldKey"]);
    project.git(&["config", "--add", "trailer.foo.key", "NewKey"]);
    let message = |key: &str| {
        format!("Subject\n\nBody.\n\nThis is one long prose line that will wrap into several lines at a narrow width\n{key}: value\n")
    };
    project.write(".git/COMMIT_EDITMSG", &message("OldKey"));
    assert_success(&project.run(&["--max-line-width=25", ".git/COMMIT_EDITMSG"]));
    assert_ne!(project.read(".git/COMMIT_EDITMSG"), message("OldKey"));

    project.write(".git/COMMIT_EDITMSG", &message("NewKey"));
    assert_success(&project.run(&["--max-line-width=25", ".git/COMMIT_EDITMSG"]));
    assert_eq!(project.read(".git/COMMIT_EDITMSG"), message("NewKey"));
}

#[test]
fn relative_git_dir_still_finds_the_comment_character() {
    let project = Project::new();
    project.init_git();
    project.git(&["config", "core.commentChar", ";"]);
    fs::create_dir_all(project.path().join("sub")).unwrap();
    let message = "Subject\n\nBody text.\n; comment   with   spaces\n";
    for (dir, git_dir, file) in [
        ("sub", "../.git", "../.git/COMMIT_EDITMSG"),
        ("", ".git", "sub/COMMIT_EDITMSG"),
    ] {
        let path = project.path().join(dir).join(file);
        fs::write(&path, message).unwrap();
        let out = project
            .command()
            .current_dir(project.path().join(dir))
            .env("GIT_DIR", git_dir)
            .arg(file)
            .output()
            .unwrap();
        assert_success(&out);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            message,
            "{git_dir} {file}"
        );
    }
}

#[test]
fn comment_prefix_ending_in_a_space_keeps_wrapping_a_tab_separated_line() {
    // `//\t` is not a `// ` comment, but `// ` is: the line is indented by a space instead of
    // being kept as written.
    let mut opts = commit_opts(30);
    opts.comment_char = "// ".to_string();
    assert_format(
        "Subject\n\n//\tlong text that should be wrapped because it is far longer than the width allows here.\n",
        "Subject\n\n // long text that should\nbe wrapped because it is far\nlonger than the width allows\nhere.\n",
        &opts,
    );
}

#[test]
fn blocks_with_attached_comment_lines_stay_as_written() {
    // Git strips a comment line touching other text and joins what remains, while CommonMark
    // reads it as part of that text, so the whole block is kept as written: the trailer after a
    // comment-separated list isn't wrapped, `#foo` isn't joined into a list item (where Git
    // would keep it), and a code span that only exists after Git's cleanup keeps its spaces.
    for (input, width) in [
        (
            "Subject\n\n- parent\n#comment\n  - child\nSigned-off-by: Alice Example <alice@example.com>\n",
            25,
        ),
        ("Subject\n\n- first  \n#foo\nsecond\n", 72),
        ("Subject\n\n`one   two\n# comment\n`\n", 72),
        ("Subject\n\n> quoted   text\n#foo\n> more   quoted\n", 72),
        // A standalone prompt tag splits the paragraph before the comment; the earlier piece
        // is kept too.
        ("Subject\n\n`one   two\n<rules>\n# comment\n`\n", 72),
        // An unclosed `<pre>` owns its trailing blank lines; keeping them in the block keeps
        // formatting idempotent.
        ("Subject\n\n#comment\n<pre>\ncode\n\n\n", 72),
    ] {
        assert_format(input, input, &commit_opts(width));
    }

    // Git's final trailer block is kept as written even when it starts with a list item: here
    // `#comment` is text (the prefix is `// `) and the list's lazy continuation is a trailer.
    let mut opts = commit_opts(20);
    opts.comment_char = "// ".to_string();
    let list_trailer =
        "Subject\n\n- parent\n#comment\n  - child\nSigned-off-by: Alice Example <alice@example.com>\n";
    assert_format(list_trailer, list_trailer, &opts);

    // The same with an earlier paragraph in the piece already split off: that piece is cut at
    // the block boundary, so the earlier paragraph is still formatted.
    assert_format(
        "Subject\n\nEarlier   paragraph.\n\n`one   two\n<rules>\n# comment\n`\n",
        "Subject\n\nEarlier paragraph.\n\n`one   two\n<rules>\n# comment\n`\n",
        &commit_opts(72),
    );

    // Blocks without comment lines around a kept block are still formatted.
    assert_format(
        "Subject\n\nFirst   para words\nmore words.\n\n- parent\n#c\n  - child\n\nLast   para   here.\n",
        "Subject\n\nFirst para words more words.\n\n- parent\n#c\n  - child\n\nLast para here.\n",
        &commit_opts(72),
    );
}

#[test]
fn multiword_comment_prefixes_pgp_after_fence_raw_html_and_leading_subjects() {
    // Multi-word `core.commentString` (`// comment`) must never wrap onto column 0 (where Git's
    // `stripspace` would delete the line) or lose leading indentation.
    let mut multi = commit_opts(11);
    multi.comment_char = "// comment".to_string();
    assert_format(
        "Subject\n\nAlpha beta // comment deleted words more.\n",
        "Subject\n\nAlpha\nbeta // comment\ndeleted\nwords more.\n",
        &multi,
    );
    assert_format(
        "Subject\n\n // comment indented words.\n",
        "Subject\n\n // comment\nindented\nwords.\n",
        &multi,
    );
    assert_format(
        "Subject\n\n//\ncomment\n",
        "Subject\n\n // comment\n",
        &multi,
    );
    let mut multi_preserve = commit_opts(72);
    multi_preserve.comment_char = "// comment".to_string();
    multi_preserve.wrap = markfmt::WrapMode::Preserve;
    assert_format(
        "Subject\n\nAlpha\n//   comment kept on preserve\n",
        "Subject\n\nAlpha\n // comment kept on preserve\n",
        &multi_preserve,
    );

    // Block-marker normalization (ATX/setext headings, list items, blockquotes, tables, rules,
    // standalone imports) must never synthesize a comment prefix or `---` patch divider.
    let mut hash_comment = commit_opts(24);
    hash_comment.comment_char = "# comment".to_string();
    assert_format(
        "Subject\n\n#   comment   heading\n\n#   normal   heading\n",
        "Subject\n\n # comment heading\n\n# normal heading\n",
        &hash_comment,
    );
    assert_format(
        "Subject\n\n  # Indented heading\n",
        "Subject\n\n # Indented heading\n",
        &commit_opts(72),
    );

    let mut dash_comment = commit_opts(22);
    dash_comment.comment_char = "- comment".to_string();
    assert_format(
        "Subject\n\n- first item\n-   comment   item   wrapping   here\n- third item\n",
        "Subject\n\n- first item\n - comment item\n   wrapping here\n- third item\n",
        &dash_comment,
    );
    let mut bare_dash = commit_opts(72);
    bare_dash.comment_char = "-".to_string();
    assert_format(
        "Subject\n\n - indented bullet\n",
        "Subject\n\n - indented bullet\n",
        &bare_dash,
    );

    // Setext `----` underline and indented `  ---` rule must not shrink/trim into a `---` Git
    // patch divider (which would hide trailers below it from `git interpret-trailers`), nor move
    // inside a preceding list whose marker was narrowed.
    assert_format(
        "Subject\n\nhi\n----\n\nBug: 123\n",
        "Subject\n\nhi\n----\n\nBug: 123\n",
        &commit_opts(72),
    );
    assert_format(
        "Subject\n\n  ---\n\nBug: 123\n",
        "Subject\n\n ---\n\nBug: 123\n",
        &commit_opts(72),
    );
    let rule_after_list = "Subject\n\n-   item\n\n    second paragraph\n  ---\n";
    common::assert_round_trip(rule_after_list, &commit_opts(72));
    assert_format(
        rule_after_list,
        "Subject\n\n- item\n\n  second paragraph\n\n ---\n",
        &commit_opts(72),
    );

    // `--hanging-indent=0` protects hard-break continuations at column 0 without adding a second
    // space after the list marker on line 0 (which would detach subsequent item paragraphs).
    let mut zero_hang = commit_opts(72);
    zero_hang.hanging_indent = Some(0);
    let list_hash_with_second_para = "Subject\n\n- #123 words\n\n  second paragraph\n";
    common::assert_round_trip(list_hash_with_second_para, &zero_hang);
    assert_format(
        list_hash_with_second_para,
        list_hash_with_second_para,
        &zero_hang,
    );
    let list_hard_break_hash = "Subject\n\n- first  \n  #123 words\n";
    common::assert_round_trip(list_hard_break_hash, &zero_hang);
    assert_format(
        list_hard_break_hash,
        "Subject\n\n- first  \n #123 words\n",
        &zero_hang,
    );

    let mut quote_comment = commit_opts(72);
    quote_comment.comment_char = "> comment".to_string();
    assert_format(
        "Subject\n\n>   comment quote\n",
        "Subject\n\n>   comment quote\n",
        &quote_comment,
    );
    let mut bare_quote = commit_opts(72);
    bare_quote.comment_char = ">".to_string();
    let quote_after_prompt_tag = "Subject\n\n-   item\n<rules>\n  > quote\n</rules>\n";
    common::assert_round_trip(quote_after_prompt_tag, &bare_quote);
    assert_format(quote_after_prompt_tag, quote_after_prompt_tag, &bare_quote);
    let quote_after_prompt_tag_blank = "Subject\n\n-   item\n<rules>\n\n  > quote\n</rules>\n";
    common::assert_round_trip(quote_after_prompt_tag_blank, &bare_quote);

    let setext_in_list_with_prompt_tag = "Subject\n\n- item\n<rules>\n  ---\n</rules>\n";
    common::assert_round_trip(setext_in_list_with_prompt_tag, &commit_opts(72));
    assert_format(
        setext_in_list_with_prompt_tag,
        setext_in_list_with_prompt_tag,
        &commit_opts(72),
    );
    let mut pipe_comment = commit_opts(72);
    pipe_comment.comment_char = "|".to_string();
    assert_format(
        "Subject\n\n  | a | b |\n  | - | - |\n",
        "Subject\n\n  | a | b |\n  | - | - |\n",
        &pipe_comment,
    );
    let table_after_wide_list =
        "Subject\n\n-   item\n\n    second paragraph\n  | a | b |\n  | - | - |\n";
    common::assert_round_trip(table_after_wide_list, &pipe_comment);
    let mut at_comment = commit_opts(72);
    at_comment.comment_char = "@".to_string();
    assert_format(
        "Subject\n\n  @docs/guide.md\n",
        "Subject\n\n  @docs/guide.md\n",
        &at_comment,
    );

    // Armored PGP signatures immediately following a closing code fence stay intact across all
    // wrap modes instead of having their base64/header lines joined or rewritten as headings,
    // and any paragraph continuation lines attached directly to `-----END ...-----` stay in the
    // same paragraph rather than detaching into indented code + a split paragraph.
    let pgp_after_fence = concat!(
        "Subject\n\n",
        "```\ncode\n```\n",
        "-----BEGIN PGP SIGNATURE-----\n",
        "iQEzBAABCAAdFiEE1234\n",
        "===\n",
        "-----END PGP SIGNATURE-----\n",
    );
    let pgp_with_attached_continuation = concat!(
        "Subject\n\n",
        "```\ncode\n```\n",
        "-----BEGIN PGP SIGNATURE-----\n",
        "iQEzBAABCAAdFiEE1234\n",
        "-----END PGP SIGNATURE-----\n",
        "    continuation\n",
        "following text\n\n",
        "Trailing   prose   still   normalizes.\n",
    );
    let pgp_expected = concat!(
        "Subject\n\n",
        "```\ncode\n```\n",
        "-----BEGIN PGP SIGNATURE-----\n",
        "iQEzBAABCAAdFiEE1234\n",
        "-----END PGP SIGNATURE-----\n",
        "    continuation\n",
        "following text\n\n",
        "Trailing prose still normalizes.\n",
    );
    for wrap in [
        markfmt::WrapMode::Always,
        markfmt::WrapMode::Never,
        markfmt::WrapMode::Preserve,
        markfmt::WrapMode::Sentence,
    ] {
        let mut opts = commit_opts(72);
        opts.wrap = wrap;
        assert_format(pgp_after_fence, pgp_after_fence, &opts);
        common::assert_round_trip(pgp_with_attached_continuation, &opts);
        assert_format(pgp_with_attached_continuation, pgp_expected, &opts);
    }

    // Unclosed raw-text HTML tags in a commit subject or front matter must stay idempotent
    // instead of inserting an extra blank line on each pass.
    let raw_subject = "Fix <code> tag handling\n\nBody   with   spaces.\n";
    assert_format(raw_subject, raw_subject, &commit_opts(72));
    assert_format(
        "---\ntitle: Fix <code> tag\n---\n\nBody   with   spaces.\n",
        "---\ntitle: Fix <code> tag\n---\n\nBody with spaces.\n",
        &common::options(72),
    );

    // Leading blank lines or template comments before the opening subject still keep the subject
    // on a single line verbatim, matching `git stripspace -s`.
    assert_format(
        "\n\nSubject line that is much longer than thirty columns and stays one line\n\nBody   text.\n",
        "Subject line that is much longer than thirty columns and stays one line\n\nBody text.\n",
        &commit_opts(30),
    );
    assert_format(
        "# Write summary below:\n\nSubject line that is much longer than thirty columns and stays one line\n\nBody   text.\n",
        "# Write summary below:\n\nSubject line that is much longer than thirty columns and stays one line\n\nBody text.\n",
        &commit_opts(30),
    );
}

#[test]
fn worktree_submodule_temp_jj_and_multiline_git_config_resolution() {
    let root = tempfile::TempDir::new().unwrap();
    let main_dir = root.path().join("main");
    let wt_dir = root.path().join("wt");
    let sub_src = root.path().join("sub_src");
    fs::create_dir_all(&main_dir).unwrap();
    fs::create_dir_all(&sub_src).unwrap();

    let git = |dir: &std::path::Path, args: &[&str]| {
        let out = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert_success(&out);
    };

    for dir in [&main_dir, &sub_src] {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.name", "Test"]);
        git(dir, &["config", "user.email", "t@e.x"]);
        git(
            dir,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
    }

    // 1. Multiline `trailer.*` values in `.git/config` must not corrupt `core.commentChar`.
    git(&main_dir, &["config", "core.commentChar", ";"]);
    git(
        &main_dir,
        &["config", "trailer.sign.cmd", "echo a\nfoo bar"],
    );
    let msg_with_bar = "Subject\n\nbar should reflow   here.\n; real comment\n";
    let main_editmsg = main_dir.join(".git/COMMIT_EDITMSG");
    fs::write(&main_editmsg, msg_with_bar).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_markfmt"))
        .current_dir(&main_dir)
        .arg(&main_editmsg)
        .output()
        .unwrap();
    assert_success(&out);
    assert_eq!(
        fs::read_to_string(&main_editmsg).unwrap(),
        "Subject\n\nbar should reflow here.\n; real comment\n"
    );

    // 2. Linked worktree uses the worktree's `.markfmt.toml` (width 25), not main's (width 60).
    fs::write(
        main_dir.join(".markfmt.toml"),
        "[commit_message]\nmax_line_width = 60\n",
    )
    .unwrap();
    git(
        &main_dir,
        &[
            "worktree",
            "add",
            "-q",
            wt_dir.to_str().unwrap(),
            "-b",
            "wt-branch",
        ],
    );
    fs::write(
        wt_dir.join(".markfmt.toml"),
        "[commit_message]\nmax_line_width = 25\n",
    )
    .unwrap();
    let wt_editmsg = main_dir.join(".git/worktrees/wt/COMMIT_EDITMSG");
    fs::write(
        &wt_editmsg,
        "Subject\n\nOne two three four five six seven eight.\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_markfmt"))
        .current_dir(&wt_dir)
        .arg(&wt_editmsg)
        .output()
        .unwrap();
    assert_success(&out);
    assert_eq!(
        fs::read_to_string(&wt_editmsg).unwrap(),
        "Subject\n\nOne two three four five\nsix seven eight.\n"
    );

    // 3. Submodule `.git/modules/sub/COMMIT_EDITMSG` uses the submodule's `.markfmt.toml`.
    git(
        &main_dir,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "-q",
            "add",
            sub_src.to_str().unwrap(),
            "sub",
        ],
    );
    let sub_dir = main_dir.join("sub");
    fs::write(
        sub_dir.join(".markfmt.toml"),
        "[commit_message]\nmax_line_width = 22\n",
    )
    .unwrap();
    let sub_editmsg = main_dir.join(".git/modules/sub/COMMIT_EDITMSG");
    fs::write(&sub_editmsg, "Subject\n\nOne two three four five six.\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_markfmt"))
        .current_dir(&sub_dir)
        .arg(&sub_editmsg)
        .output()
        .unwrap();
    assert_success(&out);
    assert_eq!(
        fs::read_to_string(&sub_editmsg).unwrap(),
        "Subject\n\nOne two three four\nfive six.\n"
    );

    // 4. External temp `*.jjdescription` (as Jujutsu creates in `$TMPDIR`) uses `cwd`'s config.
    let jj_tmp = tempfile::TempDir::new().unwrap();
    let jj_desc = jj_tmp.path().join("editor-123.jjdescription");
    fs::write(
        &jj_desc,
        "Subject\n\nOne two three four five six seven eight.\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_markfmt"))
        .current_dir(&wt_dir)
        .arg(&jj_desc)
        .output()
        .unwrap();
    assert_success(&out);
    assert_eq!(
        fs::read_to_string(&jj_desc).unwrap(),
        "Subject\n\nOne two three four five\nsix seven eight.\n"
    );
}

/// Runs `git <args>` with `input` on stdin and returns its stdout.
fn git_stdout(args: &[&str], input: &str) -> String {
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
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn trailer_block_discovery_matches_git_around_comment_blocks() {
    let long = "Custom: a valid trailer value that is long enough to need wrapping past seventy-two columns";
    let joinable = "Signed-off-by\n: Alice <alice@example.com>";
    // (comment prefix, message)
    let cases: Vec<(&str, String)> = vec![
        ("#", format!("Subject\n\nBody text.\n\n{long}\n# adjacent comment\n")),
        ("#", format!("Subject\n\nBody text.\n\n{long}\n\n# separated comment\n")),
        ("#", format!("Subject\n\nBody text.\n\n{long}\n\n# one\n\n# two\n")),
        ("#", format!("Subject\n\nBody text.\n\n{long}\n# inside\nBug: 123\n\n# after\n")),
        ("#", format!("Subject\n\nBody text.\n\n{long}\n\n# comment\n---\nnotes\n")),
        (
            "#",
            format!(
                "Subject\n\nBody text.\n\n{long}\n\n# comment\n# ------------------------ >8 ------------------------\ndiff --git a/x b/x\n"
            ),
        ),
        (";", format!("Subject\n\nBody text.\n\n{long}\n\n; custom prefix\n---\nnotes\n")),
        ("#", format!("Subject\n\nBody text.\n\n{joinable}\n\n# comment\n---\nnotes\n")),
        ("#", format!("Subject\n\nBody text.\n\n{joinable}\n\n# one\n\n# two\n")),
        (";", format!("Subject\n\nBody text.\n\n{joinable}\n\n; comment\n")),
        ("#", format!("Subject\n\nBody text.\n\n{joinable}\n# adjacent\n")),
    ];
    for (prefix, input) in cases {
        let mut opts = commit_opts(72);
        opts.comment_char = prefix.to_string();
        let once = format_markdown(&input, &opts);
        assert_eq!(
            format_markdown(&once, &opts),
            once,
            "not idempotent: {input:?}"
        );
        let cfg = format!("core.commentChar={prefix}");
        let parse = |text: &str| git_stdout(&["-c", &cfg, "interpret-trailers", "--parse"], text);
        let before = parse(&input);
        assert_eq!(
            before,
            parse(&once),
            "trailers changed: {input:?} -> {once:?}"
        );
        if !before.is_empty() {
            assert!(
                lint_with_options(&input, &opts).is_empty(),
                "accepted trailers warned: {input:?}"
            );
        }
    }
}

#[test]
fn joining_lines_never_creates_a_git_am_patch_break_or_trailer() {
    // `git am` (`mailinfo.c:patchbreak`) starts the patch at a `diff -` line.
    let mailinfo_patch = |message: &str| {
        let dir = tempfile::tempdir().unwrap();
        let (msg, patch) = (dir.path().join("msg"), dir.path().join("patch"));
        let mail = format!("From: A <a@example.com>\nSubject: s\n\n{message}");
        git_stdout(
            &["mailinfo", msg.to_str().unwrap(), patch.to_str().unwrap()],
            &mail,
        );
        fs::read_to_string(patch).unwrap()
    };
    for (input, expected) in [
        (
            "Subject\n\ndiff\n-u foo bar baz\n",
            "Subject\n\n diff -u foo bar baz\n",
        ),
        (
            "Subject\n\nSome text.  \ndiff\n-u x\n",
            "Subject\n\nSome text.  \n diff -u x\n",
        ),
        (
            "Subject\n\nCustom\n: value here\n\nmore text\n",
            "Subject\n\n Custom : value here\n\nmore text\n",
        ),
        // Collapsing whitespace would create one too.
        (
            "Subject\n\nBody.\n\ndiff  -u foo bar baz\n",
            "Subject\n\nBody.\n\n diff -u foo bar baz\n",
        ),
        (
            "Subject\n\nBody.\n\ndiff\t-u foo bar baz\n",
            "Subject\n\nBody.\n\n diff -u foo bar baz\n",
        ),
        (
            "Subject\n\nIndex:\tfoo\n\nmore text\n",
            "Subject\n\n Index: foo\n\nmore text\n",
        ),
        // Already a patch break as written: left alone.
        (
            "Subject\n\ndiff -u foo\nbar baz\n",
            "Subject\n\ndiff -u foo bar baz\n",
        ),
        (
            "Subject\n\nBody.\n\nIndex: foo\nbar baz\n\nmore text\n",
            "Subject\n\nBody.\n\nIndex: foo bar baz\n\nmore text\n",
        ),
    ] {
        let opts = commit_opts(72);
        assert_format(input, expected, &opts);
        let body = |text: &str| text.split_once("\n\n").unwrap().1.to_string();
        assert_eq!(
            mailinfo_patch(&body(input)).is_empty(),
            mailinfo_patch(&body(expected)).is_empty(),
            "{input:?}"
        );
    }
}

#[test]
fn opening_armored_block_is_not_a_subject() {
    let armor = "-----BEGIN PGP SIGNATURE-----\n\niQIzBAABCAAdFiEE long base64 line that goes on and on and on and on and on past the width\nabc\n=XyZ1\n-----END PGP SIGNATURE-----\n";
    let with_headers = "-----BEGIN PGP SIGNATURE-----\nVersion: GnuPG v2\nComment: a long comment header that goes on for a while past the width limit\n\nabc def\n-----END PGP SIGNATURE-----\n";
    for input in [
        armor.to_string(),
        with_headers.to_string(),
        format!("# leading comment\n{armor}"),
        format!("# leading comment\n\n{armor}"),
        format!("\u{feff}{armor}"),
        armor.replace('\n', "\r\n"),
    ] {
        for wrap in [
            markfmt::WrapMode::Always,
            markfmt::WrapMode::Never,
            markfmt::WrapMode::Preserve,
            markfmt::WrapMode::Sentence,
        ] {
            let mut opts = commit_opts(40);
            opts.wrap = wrap;
            assert_eq!(format_markdown(&input, &opts), input, "{wrap:?}: {input:?}");
        }
    }
}

#[test]
fn byte_order_mark_does_not_change_warnings() {
    let cases = [
        (false, "| a | b |\n| - | - |\n| `x|y` | 2 |\n"),
        (true, "| a | b |\n| - | - |\n| `x|y` | 2 |\n"),
        (true, "# ------------------------ >8 ------------------------\ndiff --git a/x b/x\nChange-Id: I1\n"),
        (true, "---\nnotes\nBug: 123\n"),
        (true, "Subject\n\nProse line.\nChange-Id: I0123456789abcdef\n"),
        (true, "Subject\n\nBody.\n\nChange-Id: I0123456789abcdef\n---\nnotes\n"),
    ];
    for (commit, input) in cases {
        let mut opts = commit_opts(72);
        opts.commit_message = commit;
        let plain = lint_with_options(input, &opts);
        let with_bom = lint_with_options(&format!("\u{feff}{input}"), &opts);
        assert_eq!(format!("{plain:?}"), format!("{with_bom:?}"), "{input:?}");
    }
}

#[test]
fn formatting_never_changes_the_trailers_git_parses() {
    let parse = |text: &str| git_stdout(&["interpret-trailers", "--parse"], text);
    for (input, expected) in [
        // Old-style `Conflicts:` footers stay as written: Git skips them only in this form.
        (
            "Subject\n\nCustom: value\n\nConflicts:\n\tfoo.md\n",
            "Subject\n\nCustom: value\n\nConflicts:\n\tfoo.md\n",
        ),
        (
            "Subject\n\nCustom: value\nConflicts:\n\tfoo.md\n",
            "Subject\n\nCustom: value\nConflicts:\n\tfoo.md\n",
        ),
        (
            "Subject\n\nBody   text.\n\nConflicts:\n\tsome/very/long/path/that/goes/on/and/on/past/the/width/limit/file.md\n",
            "Subject\n\nBody text.\n\nConflicts:\n\tsome/very/long/path/that/goes/on/and/on/past/the/width/limit/file.md\n",
        ),
        // A blank line added after a fence would start a trailer block.
        (
            "Subject\n```\ncode\n```\nCustom: value\n",
            "Subject\n\n```\ncode\n```\nCustom: value\n",
        ),
        // Removing a leading blank line would drop the trailer block.
        ("\nBug: 42\n", "\nBug: 42\n"),
        // Git reads `JJ:` as a trailer token, not a comment, so this block holds trailers too.
        (
            "\nJJ: note\n\tcode\nOther: value\n",
            "\nJJ: note\n\tcode\nOther: value\n",
        ),
        // A footer attached to a list item continues it, so the whole block stays as written.
        (
            "Subject\n\n<rules>\n-   item\n\n    second paragraph\nConflicts:\n\tfile\n",
            "Subject\n\n<rules>\n-   item\n\n    second paragraph\nConflicts:\n\tfile\n",
        ),
    ] {
        let opts = commit_opts(72);
        assert_format(input, expected, &opts);
        assert_eq!(parse(input), parse(expected), "{input:?}");
    }
}

#[test]
fn commit_tail_keeps_blank_lines_after_standalone_tags() {
    for tag in ["<rules>", "</rules>"] {
        for tail in [
            "JJ: note\n",
            "# comment\n",
            "JJ: ignore-rest\ndiff --git a/x b/x\n+a  b\n",
            "# ------------------------ >8 ------------------------\ndiff --git a/x b/x\n+a  b\n",
        ] {
            let input = format!("Subject\n\n{tag}\n\n{tail}");
            let opts = commit_opts(20);
            assert_format(&input, &input, &opts);
            common::assert_round_trip(&input, &opts);
            assert_eq!(
                git_stdout(&["interpret-trailers", "--parse"], &input),
                git_stdout(
                    &["interpret-trailers", "--parse"],
                    &format_markdown(&input, &opts)
                ),
                "{input:?}"
            );
        }
    }
}

#[test]
fn whole_message_fallback_preserves_trailing_blank_lines_in_commit_tails() {
    // Block normalization creates trailers, and neither local restoration preserves Git's
    // interpretation. The whole-message fallback must keep the detached tail verbatim.
    let head = "S\n  # h\ndiff -u\n  > q\nSigned-off-by: A\n";
    let mut opts = commit_opts(10);
    opts.comment_char = ">".to_string();
    for tail in [
        "> comment\n\n",
        "> ------------------------ >8 ------------------------\n+x\n\n",
        "JJ: ignore-rest\n+x\n\n",
    ] {
        let input = format!("{head}{tail}");
        for source in [input.clone(), input.replace('\n', "\r\n")] {
            assert_format(&source, &source, &opts);
        }
        common::assert_round_trip(&input, &opts);
        assert_eq!(
            git_stdout(
                &[
                    "-c",
                    "core.commentString=>",
                    "interpret-trailers",
                    "--parse"
                ],
                &input
            ),
            git_stdout(
                &[
                    "-c",
                    "core.commentString=>",
                    "interpret-trailers",
                    "--parse"
                ],
                &format_markdown(&input, &opts)
            ),
            "{input:?}"
        );
    }
}

#[test]
fn commit_body_preserves_heading_and_table_syntax() {
    for (input, expected) in [
        (
            "Subject\n\n- heading\n  ===\n",
            "Subject\n\n- heading\n  ===\n",
        ),
        (
            "Subject\n\n> heading\n> ===\n",
            "Subject\n\n> heading\n> ===\n",
        ),
        (
            "Subject\n\n| a | b |\n| - | - |\n| 1 | 2 |\n",
            "Subject\n\n| a | b |\n| - | - |\n| 1 | 2 |\n",
        ),
        (
            "Subject\n\na | b\n- | -\n1 | 2\n",
            "Subject\n\na | b\n- | -\n1 | 2\n",
        ),
        (
            "Subject\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n",
            "Subject\n\n| a   | b   |\n| --- | --- |\n| 1   | 2   |\n",
        ),
    ] {
        let opts = commit_opts(20);
        assert_format(input, expected, &opts);
        common::assert_round_trip(input, &opts);
        let before = git_stdout(&["stripspace", "-s"], input);
        let after = git_stdout(&["stripspace", "-s"], &format_markdown(input, &opts));
        assert_eq!(
            before.split_whitespace().collect::<Vec<_>>(),
            after.split_whitespace().collect::<Vec<_>>(),
            "{input:?}"
        );
    }
}

#[test]
fn quotes_stay_as_written_so_lazy_lines_do_not_join_them() {
    // In Markdown the second line continues the quote; in a commit message it is the author's
    // explanation, and reflowing would put it inside the quoted error.
    let input = "Fix crash\n\n> error: foo failed\nThis happens when bar is unset.\n";
    assert_format(input, input, &commit_opts(72));
    let input = "Subject\n\n> a quoted line that is long\n> and continues here\n";
    assert_format(input, input, &commit_opts(20));
}

#[test]
fn unsafe_commit_formatting_falls_back_to_the_message_as_written() {
    // Formatting this once dedents the underline and separates it, which changes the trailers
    // Git sees, so a second pass would reflow `A` and `B`: the message is kept as it is.
    let input = "A\nB\nBug: 1\n  ---\nx\n";
    assert_format(input, input, &commit_opts(72));
}

#[test]
fn partial_commit_formatting_passes_the_safety_check_with_other_text_unformatted() {
    // Reformatting the partial output would also format the second paragraph, so the
    // idempotence check must not run on it, or the selected paragraph stays unformatted.
    let path = Path::new("/virtual/COMMIT_EDITMSG");
    let only_line_3 = RangeFilter::from_line_strings(path, &["3".to_string()]).unwrap();
    let input =
        "Subject\n\nFirst   unformatted   paragraph.\n\nSecond   unformatted   paragraph.\n";
    assert_eq!(
        format_file_content(input, path, &only_line_3, &commit_opts(72)),
        "Subject\n\nFirst unformatted paragraph.\n\nSecond   unformatted   paragraph.\n"
    );
}

#[test]
fn renumbering_ordered_lists_passes_the_commit_safety_check() {
    let input = "Subject\n\n1. First   item\n3. Second   item\n";
    let mut opts = commit_opts(72);
    opts.ordered_lists = OrderedListStyle::Ascending;
    assert_format(input, "Subject\n\n1. First item\n2. Second item\n", &opts);
    opts.ordered_lists = OrderedListStyle::One;
    assert_format(input, "Subject\n\n1. First item\n1. Second item\n", &opts);
    // Without a renumbering option, a changed number would still be refused.
    assert_format(
        input,
        "Subject\n\n1. First item\n3. Second item\n",
        &commit_opts(72),
    );
}

#[test]
fn byte_order_mark_before_a_blank_line_passes_the_commit_safety_check() {
    let path = Path::new("/virtual/COMMIT_EDITMSG");
    let input = "\u{feff}\nSubject\n\nBody   with   spaces.\n";
    assert_eq!(
        format_file_content(input, path, &RangeFilter::All, &commit_opts(72)),
        "\u{feff}Subject\n\nBody with spaces.\n"
    );
}
