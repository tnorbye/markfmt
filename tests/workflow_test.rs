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
use common::{assert_success, options, Project};
use markfmt::{config::Settings, format_markdown};
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn existing_temporary_names_are_never_overwritten() {
    let project = Project::new();
    project.write("doc.md", "Needs   formatting.\n");
    let mut child = project
        .command()
        .args(["-", "doc.md"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let collision = ".markfmt-AbCd12";
    project.write(collision, "Unrelated contents.\n");
    drop(child.stdin.take());
    assert_success(&child.wait_with_output().unwrap());
    assert_eq!(project.read(collision), "Unrelated contents.\n");
    assert_eq!(project.read("doc.md"), "Needs formatting.\n");
}

#[cfg(unix)]
#[test]
fn existing_temporary_symlinks_are_never_followed() {
    let project = Project::new();
    project.write("doc.md", "Needs   formatting.\n");
    project.write("other.txt", "Unrelated contents.\n");
    let mut child = project
        .command()
        .args(["-", "doc.md"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let collision = project.path().join(".markfmt-AbCd12");
    std::os::unix::fs::symlink(project.path().join("other.txt"), &collision).unwrap();
    drop(child.stdin.take());
    assert_success(&child.wait_with_output().unwrap());
    assert_eq!(project.read("other.txt"), "Unrelated contents.\n");
    assert_eq!(project.read("doc.md"), "Needs formatting.\n");
    assert!(!fs::symlink_metadata(project.path().join("doc.md"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(fs::symlink_metadata(collision)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(unix)]
#[test]
fn failed_atomic_creation_leaves_the_original_untouched() {
    use std::os::unix::fs::PermissionsExt;
    let project = Project::new();
    project.write("doc.md", "Needs   formatting.\n");
    let original = fs::metadata(project.path()).unwrap().permissions();
    fs::set_permissions(project.path(), fs::Permissions::from_mode(0o555)).unwrap();
    // A privileged test runner can bypass directory permissions.
    if fs::write(project.path().join("probe"), "").is_ok() {
        fs::set_permissions(project.path(), original).unwrap();
        return;
    }
    let output = project.run(&["doc.md"]);
    fs::set_permissions(project.path(), original).unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(project.read("doc.md"), "Needs   formatting.\n");
}

#[test]
fn uncommitted_changes_work_before_the_first_commit() {
    let project = Project::new();
    project.init_git();
    project.write("staged.md", "First   paragraph.\n");
    project.git(&["add", "staged.md"]);
    project.write("staged.md", "First   paragraph.\n\nLater   paragraph.\n");
    project.write("untracked.md", "Untracked   paragraph.\n");
    project.write(".gitignore", "ignored.md\n");
    project.write("ignored.md", "Ignored   paragraph.\n");
    assert_success(&project.run(&["--overlaps-git-changes=uncommitted", "."]));
    assert_eq!(
        project.read("staged.md"),
        "First paragraph.\n\nLater paragraph.\n"
    );
    assert_eq!(project.read("untracked.md"), "Untracked paragraph.\n");
    assert_eq!(project.read("ignored.md"), "Ignored   paragraph.\n");
    // The HEAD mode still requires an actual commit.
    assert_eq!(
        project
            .run(&["--overlaps-git-changes=head", "staged.md"])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn deletion_only_git_changes_select_the_surviving_block() {
    for (before, after, expected) in [
        (
            "First   paragraph.\nRemove this line.\n\nUntouched   text.\n",
            "First   paragraph.\n\nUntouched   text.\n",
            "First paragraph.\n\nUntouched   text.\n",
        ),
        (
            "First   line.\n\nSecond   line.\n",
            "First   line.\nSecond   line.\n",
            "First line. Second line.\n",
        ),
        (
            "First   line.\nDeleted line.\n",
            "First   line.\n",
            "First line.\n",
        ),
    ] {
        let project = Project::new();
        project.init_git();
        project.write("doc.md", before);
        project.commit();
        project.write("doc.md", after);
        assert_success(&project.run(&["--overlaps-git-changes=uncommitted", "doc.md"]));
        assert_eq!(project.read("doc.md"), expected);
    }
}

#[test]
fn header_like_hunk_contents_do_not_change_the_git_path() {
    let project = Project::new();
    project.init_git();
    project.write(
        "doc.md",
        "Original first.\n\nUnchanged 2.\n\nUnchanged 3.\n\nUnchanged 4.\n\nOriginal last.\n",
    );
    project.commit();
    project.write(
        "doc.md",
        "++ fake.md\n\nUnchanged 2.\n\nUnchanged 3.\n\nUnchanged 4.\n\nChanged   last.\n",
    );
    assert_success(&project.run(&["--overlaps-git-changes=uncommitted", "doc.md"]));
    assert!(project.read("doc.md").ends_with("Changed last.\n"));
}

#[test]
fn print_config_resolves_precedence_without_writing() {
    let project = Project::new();
    project.write(
        ".markfmt.toml",
        "max_line_width = 30\ngreedy = true\nexclude = ['generated.md']\n",
    );
    project.write("doc.md", "Needs   formatting.\n");
    let output = project.run(&[
        "--print-config",
        "--max-line-width",
        "40",
        "--optimal",
        "doc.md",
    ]);
    assert_success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    let settings: Settings = toml::from_str(&text).unwrap();
    assert_eq!(settings.max_line_width, Some(40));
    assert_eq!(settings.greedy, Some(false));
    assert_eq!(settings.exclude, ["generated.md"]);
    assert!(text.contains(".markfmt.toml"));
    assert_eq!(project.read("doc.md"), "Needs   formatting.\n");
}

#[test]
fn explain_reports_discovery_and_configuration_exclusions() {
    let project = Project::new();
    project.write(".gitignore", "ignored.md\n");
    project.write(".markfmt.toml", "exclude = ['generated.md']\n");
    for name in ["keep.md", "ignored.md", "generated.md", "notes.txt"] {
        project.write(name, "Needs   formatting.\n");
    }
    let output = project.run(&["--explain", "."]);
    assert_success(&output);
    // Windows prints `.\keep.md`; compare with `/` separators on every platform.
    let text = String::from_utf8(output.stdout).unwrap().replace('\\', "/");
    assert!(text.contains("Included ./keep.md"), "{text}");
    assert!(
        text.contains("Skipped ./ignored.md: matched ignore rules"),
        "{text}"
    );
    assert!(
        text.contains("Skipped ./generated.md: excluded by"),
        "{text}"
    );
    assert!(
        text.contains("Skipped ./notes.txt: not a supported"),
        "{text}"
    );
    for name in ["keep.md", "ignored.md", "generated.md", "notes.txt"] {
        assert_eq!(project.read(name), "Needs   formatting.\n");
    }
}

#[test]
fn optimal_flag_overrides_config_and_last_cli_flag_wins() {
    let project = Project::new();
    project.write(".markfmt.toml", "greedy = true\nmax_line_width = 30\n");
    let input = "Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega.\n";
    for (args, optimal) in [
        (vec!["--optimal", "-"], true),
        (vec!["--greedy", "--optimal", "-"], true),
        (vec!["--optimal", "--greedy", "-"], false),
    ] {
        let output = project.stdin(&args, input);
        assert_success(&output);
        let mut opts = options(30);
        opts.optimal = optimal;
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format_markdown(input, &opts)
        );
    }
}

#[test]
fn editorconfig_directory_patterns_are_not_mistaken_for_markdown_specific() {
    let project = Project::new();
    project.write(
        ".editorconfig",
        "root = true\n[sub/**]\nmax_line_length = 10\n[sub/*.md]\nindent_size = 4\n",
    );
    project.write("sub/doc.md", "One two three four five six seven.\n");
    assert_success(&project.run(&["sub/doc.md"]));
    assert_eq!(
        project.read("sub/doc.md"),
        "One two three four five six seven.\n"
    );
}

#[test]
fn diagnostic_stdin_paths_are_included_even_when_excluded_as_files() {
    let project = Project::new();
    project.write(".markfmt.toml", "exclude = ['generated.md']\n");
    let output = project.run(&["--explain", "--stdin-filepath", "generated.md", "-"]);
    assert_success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Included "), "{text}");
    assert!(!text.contains("Skipped "), "{text}");
    assert!(text.contains(".editorconfig"), "{text}");
}

#[test]
fn oversized_editorconfig_numeric_ranges_do_not_overflow() {
    let project = Project::new();
    project.write(
        ".editorconfig",
        "root = true\n[*.{9223372036854775807..-9223372036854775808}]\nmax_line_length = 10\n",
    );
    project.write("doc.md", "One two three four five six.\n");
    assert_success(&project.run(&["doc.md"]));
    assert_eq!(project.read("doc.md"), "One two three four five six.\n");
}

#[test]
fn explain_summarizes_ignored_directories_and_skips_ignored_non_markdown() {
    let project = Project::new();
    project.write(".gitignore", "vendor/\nbuild/\n");
    project.write("keep.md", "Keep.\n");
    for i in 0..7 {
        project.write(&format!("vendor/pkg{i}/README.md"), "Vendored.\n");
    }
    project.write("build/output.bin", "binary");
    project.write("build/notes.md", "Build notes.\n");
    let output = project.run(&["--explain", "."]);
    assert_success(&output);
    // Windows prints `.\keep.md`; compare with `/` separators on every platform.
    let text = String::from_utf8(output.stdout).unwrap().replace('\\', "/");
    assert!(
        text.contains("Skipped 7 Markdown files in ./vendor: matched ignore rules"),
        "{text}"
    );
    assert!(
        text.contains("Skipped ./build/notes.md: matched ignore rules"),
        "{text}"
    );
    assert!(!text.contains("output.bin"), "{text}");
    assert!(!text.contains("pkg0"), "{text}");
}

#[test]
fn parent_components_do_not_visit_sibling_configuration() {
    let project = Project::new();
    project.write("sub/.markfmt.toml", "max_line_width = 17\n");
    project.write("sub/.editorconfig", "[*.md]\nmax_line_length = 19\n");
    project.write("top.md", "Words.\n");
    project.write("docs/note.md", "Words.\n");
    let output = project.run(&["--print-config", "sub/../top.md"]);
    assert_success(&output);
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .contains("max_line_width = 72"));
    for path in ["sub/../top.md", "sub/../virtual.md"] {
        let output = project.run(&["--print-config", "--stdin-filepath", path, "-"]);
        assert_success(&output);
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("max_line_width = 72"), "{text}");
        assert!(!text.contains("sub/.markfmt.toml"), "{text}");
    }
    let output = project.run(&["--explain", "--exclude", "docs/*.md", "sub/../docs/note.md"]);
    assert_success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("excluded by --exclude"), "{text}");
}

#[cfg(unix)]
#[test]
fn parent_components_follow_directory_symlink_semantics() {
    let project = Project::new();
    project.write("target/child/.markfmt.toml", "max_line_width = 17\n");
    project.write("target/.markfmt.toml", "max_line_width = 29\n");
    project.write("target/doc.md", "Words.\n");
    std::os::unix::fs::symlink(
        project.path().join("target/child"),
        project.path().join("alias"),
    )
    .unwrap();
    let output = project.run(&["--print-config", "alias/../doc.md"]);
    assert_success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("max_line_width = 29"), "{text}");
}

#[cfg(unix)]
#[test]
fn excludes_apply_across_symlinked_ancestors_and_parent_components() {
    let project = Project::new();
    project.write("real/.markfmt.toml", "exclude = ['target/doc.md']\n");
    project.write("real/target/child/placeholder.md", "Keep.\n");
    project.write("real/target/doc.md", "Needs   formatting.\n");
    std::os::unix::fs::symlink(project.path().join("real"), project.path().join("link")).unwrap();
    std::os::unix::fs::symlink("target/child", project.path().join("real/rel_alias")).unwrap();
    std::os::unix::fs::symlink(
        project.path().join("real/target/child"),
        project.path().join("real/abs_alias"),
    )
    .unwrap();
    std::os::unix::fs::symlink("loop", project.path().join("real/loop")).unwrap();

    for (path, cli_pattern) in [
        ("link/rel_alias/../doc.md", "link/target/doc.md"),
        ("link/abs_alias/../doc.md", "real/target/doc.md"),
    ] {
        let output = project.run(&["--explain", "--config", "link/.markfmt.toml", path]);
        assert_success(&output);
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.contains("Skipped ") && text.contains("excluded by"),
            "{text}"
        );

        let output = project.run(&["--explain", "--no-config", "--exclude", cli_pattern, path]);
        assert_success(&output);
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("excluded by --exclude"), "{text}");
    }

    // A self-referential symlink before `..` terminates cleanly instead of looping forever.
    assert_success(&project.run(&[
        "--print-config",
        "--stdin-filepath",
        "real/loop/../top.md",
        "-",
    ]));
}

#[cfg(unix)]
#[test]
fn symlinks_into_excluded_directories_are_excluded() {
    let project = Project::new();
    project.write(".markfmt.toml", "exclude = ['vendor/']\n");
    project.write("vendor/doc.md", "Needs   formatting.\n");
    // Sorts before `vendor`, so discovery keeps the link and drops the real path as a duplicate.
    std::os::unix::fs::symlink("vendor", project.path().join("a_link")).unwrap();

    assert_success(&project.run(&["."]));
    assert_eq!(project.read("vendor/doc.md"), "Needs   formatting.\n");

    let output = project.run(&["--explain", "--no-config", "--exclude", "vendor", "."]);
    assert_success(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("excluded by --exclude"), "{text}");
}

#[test]
fn explicit_config_errors_use_the_path_as_written() {
    let project = Project::new();
    project.write("bad.toml", "nope = \n");
    project.write("a.md", "Text.\n");
    for (config, message) in [
        ("bad.toml", "Invalid config `bad.toml`"),
        ("missing.toml", "Failed to read `missing.toml`"),
    ] {
        let output = project.run(&["--config", config, "a.md"]);
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(message), "{stderr}");
    }
}

#[test]
fn filter_files_applies_markdown_discovery_to_explicit_paths() {
    let project = Project::new();
    project.write("README", "# Heading\n\nNeeds   formatting.\n");
    project.write("NEWS", "Plain   text   news   entry.\n");
    project.write(
        "CHANGELOG",
        "Title\n=====\n\n.. note::\n   Needs   formatting.\n",
    );
    project.write("LICENSE.md", "Needs   formatting.\n");

    let check = project.run(&[
        "--check",
        "--filter-files",
        "NEWS",
        "CHANGELOG",
        "LICENSE.md",
    ]);
    assert_success(&check);

    assert_success(&project.run(&[
        "--filter-files",
        "README",
        "NEWS",
        "CHANGELOG",
        "LICENSE.md",
    ]));
    assert_eq!(project.read("README"), "# Heading\n\nNeeds formatting.\n");
    assert_eq!(project.read("NEWS"), "Plain   text   news   entry.\n");
    assert_eq!(
        project.read("CHANGELOG"),
        "Title\n=====\n\n.. note::\n   Needs   formatting.\n"
    );
    assert_eq!(project.read("LICENSE.md"), "Needs   formatting.\n");

    let explain = project.run(&["--explain", "--filter-files", "NEWS"]);
    assert_success(&explain);
    let text = String::from_utf8(explain.stdout).unwrap();
    assert!(
        text.contains("Skipped NEWS: not a supported Markdown documentation file"),
        "{text}"
    );

    // Pre-commit hooks pass explicit filenames, so they must enable --filter-files and text filtering.
    let hooks = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.pre-commit-hooks.yaml"
    ))
    .unwrap();
    assert_eq!(hooks.matches("--filter-files").count(), 2, "{hooks}");
    assert_eq!(hooks.matches("types: [text]").count(), 2, "{hooks}");
}

#[test]
fn pre_commit_file_patterns_match_discovery() {
    let hooks = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.pre-commit-hooks.yaml"
    ))
    .unwrap();
    let patterns: Vec<&str> = hooks
        .lines()
        .filter_map(|line| line.trim().strip_prefix("files: "))
        .collect();
    assert_eq!(patterns.len(), 2, "{hooks}");
    // The alternation group that follows `marker`, split into its members.
    let group = |pattern: &str, marker: &str| -> Vec<String> {
        let rest = &pattern[pattern.find(marker).expect(marker) + marker.len()..];
        rest[..rest.find(')').unwrap()]
            .split('|')
            .map(str::to_string)
            .collect()
    };
    for pattern in patterns {
        let mut extensions = group(pattern, r"\.(");
        extensions.retain(|ext| ext != r"md\.html");
        assert_eq!(
            extensions,
            markfmt::discovery::MARKDOWN_EXTENSIONS,
            "{pattern}"
        );
        assert!(pattern.contains(r"md\.html"), "{pattern}");
        assert_eq!(
            group(pattern, "(^|/)("),
            markfmt::discovery::EXTENSIONLESS_DOC_NAMES,
            "{pattern}"
        );
    }
}

#[test]
fn git_staged_and_head_modes_before_first_commit() {
    let project = Project::new();
    project.init_git();
    project.write("staged.md", "Staged   paragraph.\n");
    project.git(&["add", "staged.md"]);
    assert_success(&project.run(&["--overlaps-git-changes=staged", "staged.md"]));
    assert_eq!(project.read("staged.md"), "Staged paragraph.\n");

    let head = project.run(&["--overlaps-git-changes=head", "staged.md"]);
    assert_eq!(head.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&head.stderr);
    assert!(stderr.contains("no commits yet"), "{stderr}");
}

#[test]
fn git_clang_format_working_tree_and_stdout_contract() {
    let project = Project::new();
    let original = "First   unformatted.\n\nSecond   unformatted.\n";
    let expected_partial = "First unformatted.\n\nSecond   unformatted.\n";
    project.write("doc.md", original);
    project.write("clean.md", "Already clean.\n");

    // git-clang-format working-tree mode sets GIT_INDEX_FILE to a temporary clang-format-index*
    // file and passes `markfmt --lines=a:b path/to/file.md`, expecting full file output on stdout.
    for index_name in [".git/clang-format-index", ".git/clang-format-index.tmp123"] {
        let out = project
            .command()
            .env("GIT_INDEX_FILE", index_name)
            .args(["--lines=1:1", "doc.md"])
            .output()
            .unwrap();
        assert_success(&out);
        assert_eq!(String::from_utf8(out.stdout).unwrap(), expected_partial);
        assert_eq!(project.read("doc.md"), original);
    }

    // Passing `--style=...` (from clangFormat.style in git config) also triggers stdout mode.
    let styled = project.run(&["--style=file", "--lines=1:1", "doc.md"]);
    assert_success(&styled);
    assert_eq!(String::from_utf8(styled.stdout).unwrap(), expected_partial);
    assert_eq!(project.read("doc.md"), original);

    // Unchanged files must still emit their full contents on stdout rather than empty output.
    for cmd_out in [
        project
            .command()
            .env("GIT_INDEX_FILE", ".git/clang-format-index")
            .args(["--lines=1:1", "clean.md"])
            .output()
            .unwrap(),
        project.run(&["--stdout", "clean.md"]),
    ] {
        assert_success(&cmd_out);
        assert_eq!(
            String::from_utf8(cmd_out.stdout).unwrap(),
            "Already clean.\n"
        );
    }

    // Explicit `--stdout` never modifies the file on disk.
    let explicit_stdout = project.run(&["--stdout", "--lines=1:1", "doc.md"]);
    assert_success(&explicit_stdout);
    assert_eq!(
        String::from_utf8(explicit_stdout.stdout).unwrap(),
        expected_partial
    );
    assert_eq!(project.read("doc.md"), original);

    // Direct CLI `--lines` without git-clang-format markers keeps v1.0 in-place behavior.
    assert_success(&project.run(&["--lines=1:1", "doc.md"]));
    assert_eq!(project.read("doc.md"), expected_partial);

    // `-i` (`--in-place`, as passed by `clang-format-diff.py -i`) overrides `--stdout` and markers.
    let in_place = project
        .command()
        .env("GIT_INDEX_FILE", ".git/clang-format-index")
        .args(["--stdout", "doc.md", "-i", "-lines", "3:3"])
        .output()
        .unwrap();
    assert_success(&in_place);
    assert_eq!(
        project.read("doc.md"),
        "First unformatted.\n\nSecond unformatted.\n"
    );
}

#[test]
fn git_clang_format_staged_blob_and_response_file_contract() {
    let project = Project::new();
    project.write(
        ".editorconfig",
        "root = true\n[docs/*.md]\nmax_line_length = 25\n",
    );
    let blob = "One   two   three   four   five   six.\n\nUntouched   second.\n";
    let expected = "One two three four five\nsix.\n\nUntouched   second.\n";

    // Current double-dash and legacy single-dash invocations on stdin without `-`.
    for args in [
        &["--lines=1:1", "--assume-filename=docs/guide.md"][..],
        &[
            "-lines=1:1",
            "-assume-filename=docs/guide.md",
            "-style=LLVM",
        ][..],
    ] {
        let out = project.stdin(args, blob);
        assert_success(&out);
        assert_eq!(String::from_utf8(out.stdout).unwrap(), expected);
        assert!(out.stderr.is_empty(), "{:?}", out.stderr);
    }

    // Normalization also applies inside `@<filename>` response files, respecting `--`.
    project.write(
        "-weird.md",
        "| a | b |\n| - | - |\n| long cell | 2 |\n\nFirst   line.\n\nSecond   line.\n",
    );
    project.write(
        "args.txt",
        "-lines=5:5\n--align-table-columns=false\n--stdout\n--\n-weird.md\n",
    );
    let out = project.run(&["@args.txt"]);
    assert_success(&out);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "| a | b |\n| - | - |\n| long cell | 2 |\n\nFirst line.\n\nSecond   line.\n"
    );
    assert!(project.read("-weird.md").contains("First   line."));
}

#[test]
fn git_staged_shorthand_since_commits_ranges_and_default_dot() {
    let project = Project::new();
    project.init_git();
    project.git(&["checkout", "-qb", "main"]);

    project.write(
        "doc.md",
        "Legacy   one.\n\nLegacy   two.\n\nLegacy   three.\n\nLegacy   four.\n",
    );
    project.write("main_only.md", "Untouched   on   branch.\n");
    project.commit();

    // Test `--staged`, `--cached`, and `--overlaps-git-changes=cached` with omitted `<FILES>` (`.`).
    project.write(
        "doc.md",
        "Legacy   one.\n\nStaged   edit.\n\nLegacy   three.\n\nLegacy   four.\n",
    );
    project.git(&["add", "doc.md"]);
    assert_success(&project.run(&["--staged"]));
    assert_eq!(
        project.read("doc.md"),
        "Legacy   one.\n\nStaged edit.\n\nLegacy   three.\n\nLegacy   four.\n"
    );

    project.write(
        "doc.md",
        "Legacy   one.\n\nStaged edit.\n\nCached   edit.\n\nLegacy   four.\n",
    );
    project.git(&["add", "doc.md"]);
    assert_success(&project.run(&["--cached"]));
    assert_eq!(
        project.read("doc.md"),
        "Legacy   one.\n\nStaged edit.\n\nCached edit.\n\nLegacy   four.\n"
    );

    project.write(
        "doc.md",
        "Legacy   one.\n\nStaged edit.\n\nCached edit.\n\nMode   cached.\n",
    );
    project.git(&["add", "doc.md"]);
    assert_success(&project.run(&["--overlaps-git-changes=cached"]));
    assert_eq!(
        project.read("doc.md"),
        "Legacy   one.\n\nStaged edit.\n\nCached edit.\n\nMode cached.\n"
    );

    // Reset back to initial `main` state, branch `feature`, and also advance `main` independently
    // so `main..HEAD` and `main...HEAD` differ.
    project.git(&["reset", "-q", "--hard", "HEAD"]);
    project.git(&["checkout", "-qb", "feature"]);

    // Feature commit 1 (also tagged as a branch named `staged` to test `refs/heads/staged`).
    project.write(
        "doc.md",
        "Legacy   one.\n\nCommit   one   edit.\n\nLegacy   three.\n\nLegacy   four.\n",
    );
    project.commit();
    project.git(&["branch", "staged"]);

    // Feature commit 2.
    project.write(
        "doc.md",
        "Legacy   one.\n\nCommit   one   edit.\n\nCommit   two   edit.\n\nLegacy   four.\n",
    );
    project.commit();

    // Advance `main` on a separate file so `main` diverges from `feature`.
    project.git(&["checkout", "-q", "main"]);
    project.write("main_only.md", "Divergent   main   edit.\n");
    project.commit();
    project.git(&["checkout", "-q", "feature"]);
    // Put the same unformatted text on disk so if `main_only.md` were selected it would change.
    project.write("main_only.md", "Divergent   main   edit.\n");

    // `refs/heads/staged` selects the branch named `staged` (`git show`), formatting only commit 1's block!
    let show_branch = project.run(&["--diff", "--overlaps-git-changes=refs/heads/staged"]);
    assert_success(&show_branch);
    let diff_text = String::from_utf8(show_branch.stdout).unwrap();
    assert!(diff_text.contains("+Commit one edit."), "{diff_text}");
    assert!(!diff_text.contains("Commit two edit."), "{diff_text}");

    // `HEAD~1` and reflog syntax `HEAD@{0 minutes ago}` run `git show`.
    let show_prev = project.run(&["--diff", "--overlaps-git-changes=HEAD~1"]);
    assert_success(&show_prev);
    let prev_text = String::from_utf8(show_prev.stdout).unwrap();
    assert!(prev_text.contains("+Commit one edit."), "{prev_text}");
    assert!(!prev_text.contains("Commit two edit."), "{prev_text}");

    let show_reflog = project.run(&["--diff", "--overlaps-git-changes=HEAD@{0 minutes ago}"]);
    assert_success(&show_reflog);
    let reflog_text = String::from_utf8(show_reflog.stdout).unwrap();
    assert!(reflog_text.contains("+Commit two edit."), "{reflog_text}");
    assert!(!reflog_text.contains("Commit one edit."), "{reflog_text}");

    // Three-dot `main...HEAD` selects both feature commits on `doc.md` while ignoring `main`'s
    // divergent commit on `main_only.md`. Two-dot `main..HEAD` sees the difference in `main_only.md` too.
    let three_dot = project.run(&["--diff", "--overlaps-git-changes=main...HEAD"]);
    assert_success(&three_dot);
    let three_text = String::from_utf8(three_dot.stdout).unwrap();
    assert!(three_text.contains("+Commit one edit."), "{three_text}");
    assert!(three_text.contains("+Commit two edit."), "{three_text}");
    assert!(!three_text.contains("main_only.md"), "{three_text}");

    let two_dot = project.run(&["--diff", "--overlaps-git-changes=main..HEAD"]);
    assert_success(&two_dot);
    let two_text = String::from_utf8(two_dot.stdout).unwrap();
    assert!(two_text.contains("main_only.md"), "{two_text}");

    // `--since HEAD~2` formats committed changes since `HEAD~2`, unstaged working-tree edits, and
    // untracked files, while leaving `Legacy   one.` untouched.
    project.write(
        "doc.md",
        "Legacy   one.\n\nCommit   one   edit.\n\nCommit   two   edit.\n\nWorking   tree   edit.\n",
    );
    project.write("untracked.md", "Brand   new   note.\n");
    assert_success(&project.run(&["--since", "HEAD~2"]));
    assert_eq!(
        project.read("doc.md"),
        "Legacy   one.\n\nCommit one edit.\n\nCommit two edit.\n\nWorking tree edit.\n"
    );
    assert_eq!(project.read("untracked.md"), "Brand new note.\n");

    // Reject open-ended ranges, leading-`-` revisions, and unknown revisions cleanly.
    for bad in [
        &["--overlaps-git-changes=main....HEAD"][..],
        &["--overlaps-git-changes=-bad"][..],
        &["--since=-bad"][..],
        &["--overlaps-git-changes=nonexistent_rev"][..],
        &["--since=nonexistent_rev"][..],
    ] {
        let out = project.run(bad);
        assert_eq!(out.status.code(), Some(2), "{bad:?}: {out:?}");
    }
}

#[test]
fn clang_format_stdout_mode_echoes_excluded_files_and_accepts_diff_flags() {
    let project = Project::new();
    let original = "First   unformatted.\n\nSecond   unformatted.\n";
    project.write(".markfmt.toml", "exclude = [\"vendor/**\"]\n");
    project.write("vendor/doc.md", original);
    project.write("doc.md", original);

    // git-clang-format turns empty output into an empty file, so excluded files must be echoed.
    let out = project
        .command()
        .env("GIT_INDEX_FILE", ".git/clang-format-index-abc")
        .args(["--lines=1:1", "vendor/doc.md"])
        .output()
        .unwrap();
    assert_success(&out);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), original);
    assert_eq!(project.read("vendor/doc.md"), original);

    // Outside stdout mode, excluded files are still skipped.
    let out = project.run(&["--lines=1:1", "vendor/doc.md"]);
    assert_success(&out);
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("unformatted"),
        "{out:?}"
    );
    assert_eq!(project.read("vendor/doc.md"), original);

    // clang-format-diff.py passes `--style`, `--fallback-style`, and `--sort-includes` (in either
    // dash style); with `--lines` they select stdout mode, and `-i` still edits in place.
    for args in [
        &["doc.md", "--lines", "1:1", "--style", "file"][..],
        &["doc.md", "--lines", "1:1", "--fallback-style", "none"][..],
        &["doc.md", "-lines", "1:1", "-sort-includes"][..],
        &["doc.md", "-lines", "1:1", "-sort-includes=true"][..],
        &["doc.md", "-lines", "1:1", "--sort-includes=false"][..],
        &["-sort-includes=false", "doc.md", "-lines", "1:1"][..],
    ] {
        let out = project.run(args);
        assert_success(&out);
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "First unformatted.\n\nSecond   unformatted.\n",
            "{args:?}"
        );
        assert_eq!(project.read("doc.md"), original, "{args:?}");
    }
    let out = project.run(&["doc.md", "-i", "--sort-includes=false", "--lines", "3:3"]);
    assert_success(&out);
    assert_eq!(
        project.read("doc.md"),
        "First   unformatted.\n\nSecond unformatted.\n"
    );

    // Like clang-format, a style flag without files reads stdin.
    for flag in ["-style=file", "-sort-includes=false"] {
        let out = project.stdin(&[flag], "Piped   text.\n");
        assert_success(&out);
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "Piped text.\n",
            "{flag}"
        );
    }
}

#[test]
fn git_commit_ranges_follow_later_edits_and_merges() {
    let project = Project::new();
    project.init_git();
    project.git(&["checkout", "-qb", "main"]);
    project.write("doc.md", "Old   one.\n\nOld   two.\n");
    project.commit();
    project.write("doc.md", "Old   one.\n\nNew   two.\n");
    project.commit();
    // A later commit inserts blocks above the line `HEAD~1` changed.
    project.write(
        "doc.md",
        "Top   a.\n\nTop   b.\n\nOld   one.\n\nNew   two.\n",
    );
    project.commit();

    for mode in ["HEAD~1", "HEAD~2..HEAD~1", "HEAD~2...HEAD~1"] {
        let out = project.run(&["--diff", "--overlaps-git-changes", mode]);
        assert_success(&out);
        let diff = String::from_utf8(out.stdout).unwrap();
        assert!(diff.contains("+New two."), "{mode}: {diff}");
        assert!(!diff.contains("+Top"), "{mode}: {diff}");
        assert!(!diff.contains("+Old one."), "{mode}: {diff}");
    }

    // Staged line numbers refer to the index; unstaged edits above them move them down.
    project.write(
        "doc.md",
        "Top   a.\n\nTop   b.\n\nOld   one.\n\nNew   two.\n\nStaged   three.\n",
    );
    project.git(&["add", "doc.md"]);
    project.write(
        "doc.md",
        "Unstaged   top.\n\nTop   a.\n\nTop   b.\n\nOld   one.\n\nNew   two.\n\nStaged   three.\n",
    );
    let out = project.run(&["--diff", "--staged"]);
    assert_success(&out);
    let diff = String::from_utf8(out.stdout).unwrap();
    assert!(diff.contains("+Staged three."), "{diff}");
    assert!(!diff.contains("+Unstaged top."), "{diff}");
    assert!(!diff.contains("+New two."), "{diff}");
    project.git(&["reset", "-q", "--hard", "HEAD"]);

    // A merge commit is compared with its first parent, including its own resolution edits
    // (`git show` would print a combined diff instead).
    project.git(&["checkout", "-qb", "side"]);
    project.write("side.md", "Side   file.\n");
    project.commit();
    project.git(&["checkout", "-q", "main"]);
    project.git(&[
        "-c",
        "commit.gpgsign=false",
        "merge",
        "-q",
        "--no-ff",
        "--no-commit",
        "side",
    ]);
    project.write(
        "doc.md",
        "Top   a.\n\nTop   b.\n\nOld   one.\n\nMerge   fix.\n",
    );
    project.commit();
    let out = project.run(&["--diff", "--overlaps-git-changes=HEAD"]);
    assert_success(&out);
    let diff = String::from_utf8(out.stdout).unwrap();
    assert!(diff.contains("+Merge fix."), "{diff}");
    assert!(diff.contains("+Side file."), "{diff}");
    assert!(!diff.contains("+Top"), "{diff}");
}

#[test]
fn git_change_filters_ignore_textconv_and_external_diff() {
    let project = Project::new();
    project.init_git();
    project.write(".gitattributes", "*.md diff=custom\n");
    project.write("doc.md", "Committed   one.\n");
    project.commit();
    // Either setting would fail (or change line numbers) if markfmt let git use it.
    project.git(&[
        "config",
        "diff.custom.textconv",
        "markfmt-test-missing-textconv",
    ]);
    project.git(&["config", "diff.external", "markfmt-test-missing-diff"]);
    project.write("doc.md", "Committed   one.\n\nAdded   two.\n");
    project.git(&["add", "doc.md"]);
    for mode in ["uncommitted", "staged"] {
        let out = project.run(&["--diff", "--overlaps-git-changes", mode]);
        assert_success(&out);
        let diff = String::from_utf8(out.stdout).unwrap();
        assert!(diff.contains("+Added two."), "{mode}: {diff}");
        assert!(!diff.contains("+Committed one."), "{mode}: {diff}");
    }
}

#[test]
fn excluded_files_are_listed_and_echoed_only_for_clang_format_calls() {
    let project = Project::new();
    let original = "First   unformatted.\n\nSecond   unformatted.\n";
    project.write(".markfmt.toml", "exclude = [\"docs/vendor/**\"]\n");
    project.write("docs/vendor/skip.md", original);
    project.write("docs/doc.md", "Kept   doc.\n");

    // `git-clang-format` first asks which changed files are ignored (possibly none), without
    // reading stdin, and drops them before formatting.
    let out = project.run(&["-list-ignored"]);
    assert_success(&out);
    assert!(out.stdout.is_empty(), "{out:?}");
    let out = project.run(&["-list-ignored", "docs/doc.md", "docs/vendor/skip.md"]);
    assert_success(&out);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "docs/vendor/skip.md\n"
    );
    let out = project.run(&["--list-ignored", "--exclude=docs/doc.md", "docs/doc.md"]);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "docs/doc.md\n");
    assert_eq!(project.read("docs/doc.md"), "Kept   doc.\n");

    // Stdin ignores excludes, with or without `--lines`, as editors expect.
    for args in [
        &["--assume-filename=docs/vendor/skip.md"][..],
        &["--lines=1:3", "--assume-filename=docs/vendor/skip.md"][..],
    ] {
        let out = project.stdin(args, original);
        assert_success(&out);
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "First unformatted.\n\nSecond unformatted.\n",
            "{args:?}"
        );
    }

    // `git-clang-format --whole-file` passes no `--lines`; its index marker still selects stdout,
    // and an excluded named file is echoed rather than skipped.
    for (file, expected) in [
        ("docs/doc.md", "Kept doc.\n"),
        ("docs/vendor/skip.md", original),
    ] {
        let out = project
            .command()
            .env("GIT_INDEX_FILE", ".git/clang-format-index-abc")
            .arg(file)
            .output()
            .unwrap();
        assert_success(&out);
        assert_eq!(String::from_utf8(out.stdout).unwrap(), expected, "{file}");
    }
    assert_eq!(project.read("docs/doc.md"), "Kept   doc.\n");

    // Folder walks in `--stdout` mode leave excluded files out entirely.
    let out = project.run(&["--stdout", "-v", "docs"]);
    assert_success(&out);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "Kept doc.\n");
    assert!(String::from_utf8_lossy(&out.stderr).contains("Skipping excluded"));

    // Even when a folder walk finds only one (excluded) file, nothing is echoed.
    let out = project.run(&["--stdout", "docs/vendor"]);
    assert_success(&out);
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("unformatted"),
        "{out:?}"
    );

    // A single echoed file is not also reported as skipped.
    let out = project.run(&[
        "--stdout",
        "-v",
        "--style=file",
        "--lines=1:1",
        "docs/vendor/skip.md",
    ]);
    assert_success(&out);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), original);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("Skipping excluded"));
}

#[test]
fn git_ranges_drop_deleted_lines_and_follow_renames() {
    let project = Project::new();
    project.init_git();
    project.write("doc.md", "Old   one.\n\nOld   two.\n\nOld   three.\n");
    project.commit();

    // A staged paragraph deleted again in the working tree selects nothing, not its neighbors.
    project.write(
        "doc.md",
        "Old   one.\n\nStaged   new.\n\nOld   two.\n\nOld   three.\n",
    );
    project.git(&["add", "doc.md"]);
    project.write("doc.md", "Old   one.\n\nOld   two.\n\nOld   three.\n");
    let out = project.run(&["--diff", "--staged"]);
    assert_success(&out);
    assert!(out.stdout.is_empty(), "{out:?}");

    // When only part of a staged change is deleted, the surviving part is still formatted.
    project.write(
        "doc.md",
        "Old   one.\n\nStaged   a.\n\nStaged   b.\n\nOld   two.\n\nOld   three.\n",
    );
    project.git(&["add", "doc.md"]);
    project.write(
        "doc.md",
        "Old   one.\n\nStaged   b.\n\nOld   two.\n\nOld   three.\n",
    );
    let out = project.run(&["--diff", "--staged"]);
    assert_success(&out);
    let diff = String::from_utf8(out.stdout).unwrap();
    assert!(diff.contains("+Staged b."), "{diff}");
    assert!(!diff.contains("+Old"), "{diff}");
    project.git(&["reset", "-q", "--hard", "HEAD"]);

    // Ranges from a commit follow a later rename, even with `diff.renames = false` configured.
    project.write("doc.md", "Old   one.\n\nNew   two.\n\nOld   three.\n");
    project.commit();
    project.git(&["mv", "doc.md", "moved.md"]);
    project.commit();
    project.git(&["config", "diff.renames", "false"]);
    let out = project.run(&["--diff", "--overlaps-git-changes=HEAD~1"]);
    assert_success(&out);
    let diff = String::from_utf8(out.stdout).unwrap();
    assert!(diff.contains("moved.md"), "{diff}");
    assert!(diff.contains("+New two."), "{diff}");
    assert!(!diff.contains("+Old"), "{diff}");

    // A file deleted since the commit is simply skipped.
    project.git(&["rm", "-q", "moved.md"]);
    let out = project.run(&["--diff", "--overlaps-git-changes=HEAD~1"]);
    assert_success(&out);
    assert!(out.stdout.is_empty(), "{out:?}");

    // Out-of-range reflog entries fail with a complete message.
    let out = project.run(&["--overlaps-git-changes=HEAD@{99999}"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Unknown git revision"), "{stderr}");
}

#[test]
fn git_ranges_split_at_insertions_and_match_stdin_contents() {
    let project = Project::new();
    project.init_git();
    project.write("doc.md", "Base   one.\n\nBase   two.\n\nBase   three.\n");
    project.commit();

    // A staged addition spanning two paragraphs, with an unstaged paragraph inserted between.
    project.write(
        "doc.md",
        "Base   one.\n\nStaged   a.\n\nStaged   b.\n\nBase   two.\n\nBase   three.\n",
    );
    project.git(&["add", "doc.md"]);
    project.write(
        "doc.md",
        "Base   one.\n\nStaged   a.\n\nUnstaged   mid.\n\nStaged   b.\n\nBase   two.\n\nBase   three.\n",
    );
    let out = project.run(&["--diff", "--staged"]);
    assert_success(&out);
    let diff = String::from_utf8(out.stdout).unwrap();
    assert!(diff.contains("+Staged a."), "{diff}");
    assert!(diff.contains("+Staged b."), "{diff}");
    assert!(!diff.contains("+Unstaged mid."), "{diff}");
    assert!(!diff.contains("+Base"), "{diff}");

    // Piping the staged blob uses its own line numbers, not the shifted working-tree file.
    project.write(
        "doc.md",
        "Top   a.\n\nTop   b.\n\nBase   one.\n\nStaged   a.\n\nStaged   b.\n\nBase   two.\n\nBase   three.\n",
    );
    let staged_blob = "Base   one.\n\nStaged   a.\n\nStaged   b.\n\nBase   two.\n\nBase   three.\n";
    let out = project.stdin(
        &[
            "--overlaps-git-changes=staged",
            "--stdin-filepath",
            "doc.md",
            "-",
        ],
        staged_blob,
    );
    assert_success(&out);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "Base   one.\n\nStaged a.\n\nStaged b.\n\nBase   two.\n\nBase   three.\n"
    );

    // An unsaved CRLF editor buffer with an extra paragraph formats its own uncommitted changes
    // relative to HEAD while ignoring unrelated edits on disk and keeping untouched blocks intact.
    let buffer = "Base   one.\r\n\r\nBuffer   new.\r\n\r\nBase   two.\r\n\r\nBase   three.\r\n";
    let expected_buffer =
        "Base   one.\r\n\r\nBuffer new.\r\n\r\nBase   two.\r\n\r\nBase   three.\r\n";
    project.git(&["reset", "-q", "--hard", "HEAD"]);
    project.write(
        "doc.md",
        "Base   one.\n\nBase   two.\n\nBase   three.\n\nDisk   edit.\n",
    );
    for args in [
        &[
            "--overlaps-git-changes=uncommitted",
            "--stdin-filepath",
            "doc.md",
            "-",
        ][..],
        &["--since", "HEAD", "--stdin-filepath", "doc.md", "-"][..],
    ] {
        let out = project.stdin(args, buffer);
        assert_success(&out);
        assert_eq!(String::from_utf8(out.stdout).unwrap(), expected_buffer);
    }

    // An unsaved buffer for a brand-new file not yet on disk is formatted in full.
    let new_buf = project.stdin(
        &[
            "--overlaps-git-changes=uncommitted",
            "--stdin-filepath",
            "docs/unsaved.md",
            "-",
        ],
        "Brand   new   buffer.\n",
    );
    assert_success(&new_buf);
    assert_eq!(
        String::from_utf8(new_buf.stdout).unwrap(),
        "Brand new buffer.\n"
    );
}

#[test]
fn git_filters_resist_config_and_unusual_paths() {
    let project = Project::new();
    project.init_git();
    project.write("doc.md", "Base   one.\n\nKeep   me.\n\nBase   three.\n");
    project.commit();

    // `diff.interHunkContext` must not merge two edits around an untouched paragraph.
    project.git(&["config", "diff.interHunkContext", "10"]);
    project.write("doc.md", "Edit   one.\n\nKeep   me.\n\nEdit   three.\n");
    project.git(&["add", "doc.md"]);
    let out = project.run(&["--diff", "--staged"]);
    assert_success(&out);
    let diff = String::from_utf8(out.stdout).unwrap();
    assert!(diff.contains("+Edit one."), "{diff}");
    assert!(!diff.contains("+Keep me."), "{diff}");
    project.commit();

    // The root commit is selected even with `log.showRoot = false`.
    project.git(&["config", "log.showRoot", "false"]);
    let out = Command::new("git")
        .current_dir(project.path())
        .args(["rev-list", "--max-parents=0", "HEAD"])
        .output()
        .unwrap();
    let root = String::from_utf8(out.stdout).unwrap().trim().to_string();
    let out = project.run(&["--dry-run", &format!("--overlaps-git-changes={root}")]);
    assert_success(&out);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("doc.md"),
        "{out:?}"
    );

    // A file named after a commit ID does not make the revision ambiguous.
    let head = Command::new("git")
        .current_dir(project.path())
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let head = String::from_utf8(head.stdout).unwrap().trim().to_string();
    project.write(&head, "");
    project.write(
        "doc.md",
        "Edit   one.\n\nKeep   me.\n\nEdit   three.\n\nNew   four.\n",
    );
    let out = project.run(&["--dry-run", "--since", "HEAD"]);
    assert_success(&out);
    fs::remove_file(project.path().join(&head)).unwrap();
    project.git(&["checkout", "-q", "doc.md"]);

    // Names that start with a space are matched exactly.
    project.write(" space.md", "Lead   space.\n");
    project.git(&["add", " space.md"]);
    let out = project.run(&["--dry-run", "--staged"]);
    assert_success(&out);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(" space.md"),
        "{out:?}"
    );
    project.git(&["reset", "-q", "--hard", "HEAD"]);

    // Retargeting a symlink never selects lines in the document it points to.
    #[cfg(unix)]
    {
        project.write("target.md", "Target   one.\n\nTarget   two.\n");
        std::os::unix::fs::symlink("doc.md", project.path().join("link.md")).unwrap();
        project.commit();
        fs::remove_file(project.path().join("link.md")).unwrap();
        std::os::unix::fs::symlink("target.md", project.path().join("link.md")).unwrap();
        project.git(&["add", "link.md"]);
        let out = project.run(&["--dry-run", "--staged"]);
        assert_success(&out);
        assert!(out.stdout.is_empty(), "{out:?}");
    }
}

#[test]
fn stdin_git_ranges_follow_renames_and_wrapper_excludes() {
    let project = Project::new();
    project.init_git();
    project.write(".markfmt.toml", "exclude = [\"vendor/**\"]\n");
    project.write("vendor/doc.md", "Vendor   one.\n");
    project.write("doc.md", "Base   one.\n\nBase   two.\n\nBase   three.\n");
    project.commit();
    project.write("doc.md", "Base   one.\n\nEdit   two.\n\nBase   three.\n");
    project.commit();
    project.git(&["mv", "doc.md", "renamed.md"]);
    project.commit();

    // Ranges from before a rename reach stdin (named after the new file) in its own lines.
    let buffer = "Base   one.\n\nExtra   para.\n\nEdit   two.\n\nBase   three.\n";
    let out = project.stdin(
        &[
            "--overlaps-git-changes=HEAD~1",
            "--stdin-filepath",
            "renamed.md",
            "-",
        ],
        buffer,
    );
    assert_success(&out);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "Base   one.\n\nExtra   para.\n\nEdit two.\n\nBase   three.\n"
    );

    // Stdin and the file it names cannot share one git-filtered run.
    let out = project.stdin(
        &[
            "--diff",
            "--overlaps-git-changes=HEAD~1",
            "--stdin-filepath",
            "renamed.md",
            "-",
            "renamed.md",
        ],
        buffer,
    );
    assert_eq!(out.status.code(), Some(2), "{out:?}");

    // Older git-clang-format versions (no `-list-ignored`) pipe excluded blobs in `--staged`
    // and two-commit modes; under its index marker they are echoed unchanged.
    let mut child = project
        .command()
        .env("GIT_INDEX_FILE", ".git/clang-format-index")
        .args(["-lines=1:1", "-assume-filename=vendor/doc.md"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"Vendor   one.\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_success(&out);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "Vendor   one.\n");
}

#[test]
fn git_diff_opts_cannot_widen_selected_changes() {
    let project = Project::new();
    project.init_git();
    project.write("doc.md", "Base   one.\n\nBase   two.\n\nBase   three.\n");
    project.commit();
    project.write("doc.md", "Base   one.\n\nEdit   two.\n\nBase   three.\n");
    project.git(&["add", "doc.md"]);
    let out = project
        .command()
        .env("GIT_DIFF_OPTS", "--unified=3")
        .args(["--diff", "--staged"])
        .output()
        .unwrap();
    assert_success(&out);
    let diff = String::from_utf8(out.stdout).unwrap();
    assert!(diff.contains("+Edit two."), "{diff}");
    assert!(!diff.contains("+Base"), "{diff}");
}

#[test]
fn repeated_staged_formatting_never_reaches_unstaged_paragraphs() {
    let long =
        "Staged paragraph that is long enough that formatting has to wrap it onto two lines.";
    // The unstaged paragraph is either new text or an exact copy of the staged one, which git
    // would otherwise match to the index once the staged copy is formatted.
    for (staged, unstaged) in [
        ("Staged   paragraph.", "Unstaged   paragraph."),
        (long, "Unstaged   paragraph."),
        ("Staged   paragraph.", "Staged   paragraph."),
        (long, long),
    ] {
        let project = Project::new();
        project.init_git();
        project.write("doc.md", "Original.\n");
        project.commit();
        project.write("doc.md", &format!("Original.\n\n{staged}\n"));
        project.git(&["add", "doc.md"]);
        let original = format!("Original.\n\n{staged}\n\n{unstaged}\n");
        project.write("doc.md", &original);

        // Once formatted, the staged paragraph no longer matches the index line for line; the
        // second run (and `--check`) must still select only the staged paragraph.
        assert_success(&project.run(&["--staged", "doc.md"]));
        let first = project.read("doc.md");
        assert!(first.ends_with(&format!("\n\n{unstaged}\n")), "{first}");
        assert_ne!(first, original);
        let check = project.run(&["--check", "--staged", "doc.md"]);
        assert_success(&check);
        assert_success(&project.run(&["--staged", "doc.md"]));
        assert_eq!(project.read("doc.md"), first, "{staged} / {unstaged}");
    }
}

#[test]
fn rewritten_staged_paragraph_does_not_select_a_paragraph_added_after_it() {
    let added =
        "Added paragraph that is long enough that formatting would have to wrap it onto two lines.";
    let project = Project::new();
    project.init_git();
    project.write("doc.md", "# Title\n\nAlpha beta.\n\nOmega.\n");
    project.commit();
    project.write("doc.md", "# Title\n\nAlpha gamma.\n\nOmega.\n");
    project.git(&["add", "doc.md"]);
    // No word of the staged paragraph survives, so its replacement and the new paragraph after
    // it are one replaced span in the word diff.
    project.write(
        "doc.md",
        &format!("# Title\n\nDelta   epsilon.\n\n{added}\n\nOmega.\n"),
    );

    assert_success(&project.run(&["--staged", "doc.md"]));
    assert_eq!(
        project.read("doc.md"),
        format!("# Title\n\nDelta epsilon.\n\n{added}\n\nOmega.\n")
    );

    // Paragraphs paired in order still follow a staged paragraph that was rewritten in place.
    project.write("doc.md", "# Title\n\nAlpha gamma.\n\nOmega.\n");
    project.git(&["add", "doc.md"]);
    project.write("doc.md", "# Title\n\nZeta   eta.\n\nOmega.\n");
    assert_success(&project.run(&["--staged", "doc.md"]));
    assert_eq!(project.read("doc.md"), "# Title\n\nZeta eta.\n\nOmega.\n");
}

#[test]
fn relative_git_location_variables_resolve_against_the_working_directory() {
    let project = Project::new();
    project.init_git();
    project.write("docs/guide.md", "Hello   world.\n");
    project.commit();
    project.write("docs/guide.md", "Hello   there.\n");
    project.git(&["add", "docs/guide.md"]);
    fs::create_dir_all(project.path().join("sub")).unwrap();

    // Git sets these, relative to its own directory, for `git --git-dir=.git` and in hooks;
    // markfmt runs git in other directories with `-C`.
    for (dir, vars) in [
        ("", vec![("GIT_DIR", ".git")]),
        (
            "",
            vec![
                ("GIT_DIR", ".git"),
                ("GIT_WORK_TREE", "."),
                ("GIT_INDEX_FILE", ".git/index"),
            ],
        ),
        ("sub", vec![("GIT_INDEX_FILE", "../.git/index")]),
        ("sub", vec![("GIT_DIR", "../.git")]),
    ] {
        let target = if dir.is_empty() { "docs" } else { "../docs" };
        let out = project
            .command()
            .current_dir(project.path().join(dir))
            .envs(vars.iter().copied())
            .args(["--check", "--staged", target])
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(1),
            "{vars:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        // `--check` reports on stderr.
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("Would reformat"),
            "{vars:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn git_dir_of_a_bare_repository_does_not_redirect_another_checkout() {
    let project = Project::new();
    project.git(&["init", "-q", "--bare", "bare.git"]);
    project.write("repo/doc.md", "Hello.\n");
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(project.path().join("repo"))
            .args(args)
            .output()
            .unwrap();
        assert_success(&out);
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "Formatter Test"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["add", "."]);
    git(&[
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "Test fixture",
    ]);
    project.write("repo/doc.md", "Hello   there.\n");
    git(&["add", "doc.md"]);

    // Git finds no worktree for a bare repository, but its `GIT_DIR` still must not apply to
    // a checkout outside it.
    let out = project
        .command()
        .current_dir(project.path().join("bare.git"))
        .env("GIT_DIR", ".")
        .args(["--check", "--staged", "../repo"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("Would reformat"));
}

#[test]
fn git_filtered_runs_never_read_unselected_files() {
    let project = Project::new();
    project.init_git();
    project.write("touched.md", "Touched.\n");
    // Not UTF-8, and a lint warning: neither may matter when the file has no selected changes.
    fs::write(project.path().join("untouched_bad.md"), b"\xff\xfe bad\n").unwrap();
    project.write("untouched_warn.md", "| a | b |\n|---|---|\n| `x|y` | z |\n");
    project.commit();
    project.write("touched.md", "Touched   again.\n");
    project.git(&["add", "touched.md"]);

    let out = project.run(&["--check", "--staged"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Would reformat touched.md"), "{stderr}");
    assert!(!stderr.contains("untouched"), "{stderr}");
    assert!(!stderr.contains("could not be processed"), "{stderr}");
}

#[test]
fn git_filtered_stdin_works_when_its_folder_is_missing() {
    let project = Project::new();
    project.init_git();
    project.write("doc.md", "Committed.\n");
    project.commit();
    project.write("docs/new.md", "New   doc.\n");
    project.git(&["add", "docs/new.md"]);
    fs::remove_dir_all(project.path().join("docs")).unwrap();

    let out = project.stdin(
        &["--staged", "--stdin-filepath", "docs/new.md", "-"],
        "New   doc.\n",
    );
    assert_success(&out);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "New doc.\n");

    // Also works when `--stdin-filepath` traverses a symlinked ancestor directory (such as `/tmp`
    // -> `/private/tmp` on macOS) whose child `docs/` folder does not exist on disk.
    #[cfg(unix)]
    {
        let outer = tempfile::TempDir::new().unwrap();
        let alias = outer.path().join("repo-link");
        std::os::unix::fs::symlink(project.path(), &alias).unwrap();
        let missing_via_link = alias.join("docs/new.md");
        let out = project.stdin(
            &[
                "--staged",
                "--stdin-filepath",
                missing_via_link.to_str().unwrap(),
                "-",
            ],
            "New   doc.\n",
        );
        assert_success(&out);
        assert_eq!(String::from_utf8(out.stdout).unwrap(), "New doc.\n");
    }
}

#[test]
fn lines_intersect_git_filters_and_external_symlinks_resolve_repo() {
    let project = Project::new();
    project.init_git();
    project.write("doc.md", "Para   one.\n\nPara   two.\n\nPara   three.\n");
    project.commit();
    // Stage edits to paragraphs 1 and 2; leave paragraph 3 unstaged-edited too.
    project.write(
        "doc.md",
        "Staged   one.\n\nStaged   two.\n\nPara   three.\n",
    );
    project.git(&["add", "doc.md"]);
    project.write(
        "doc.md",
        "Staged   one.\n\nStaged   two.\n\nUnstaged   three.\n",
    );

    // `--lines 3:5 --staged` formats only blocks overlapping BOTH `--lines` (paras 2 & 3) and
    // `--staged` (paras 1 & 2) -> only paragraph 2!
    assert_success(&project.run(&["--lines=3:5", "--staged", "doc.md"]));
    assert_eq!(
        project.read("doc.md"),
        "Staged   one.\n\nStaged two.\n\nUnstaged   three.\n"
    );

    // A symlink outside any git repository pointing to `doc.md` resolves `doc.md`'s repository.
    #[cfg(unix)]
    {
        let outside = Project::new();
        let link = outside.path().join("external.md");
        std::os::unix::fs::symlink(project.path().join("doc.md"), &link).unwrap();
        assert_success(&outside.run(&["--staged", link.to_str().unwrap()]));
        assert_eq!(
            project.read("doc.md"),
            "Staged one.\n\nStaged two.\n\nUnstaged   three.\n"
        );
    }
}

#[test]
fn stdin_and_disk_ranges_stay_independent_across_renames() {
    let project = Project::new();
    project.init_git();
    project.write("a.md", "Base   one.\n");
    project.commit();
    project.write("a.md", "Base   one.\n\nEdit   two.\n");
    project.commit();
    project.git(&["mv", "a.md", "b.md"]);

    // Disk only: the commit's ranges follow the rename.
    let out = project.run(&["--overlaps-git-changes=HEAD", "--stdout", "b.md"]);
    assert_success(&out);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "Base   one.\n\nEdit two.\n"
    );

    // Stdin only, named after the old path, with a paragraph inserted above.
    let buffer = "New   para.\n\nBase   one.\n\nEdit   two.\n";
    let stdin_only = "New   para.\n\nBase   one.\n\nEdit two.\n";
    let out = project.stdin(
        &[
            "--overlaps-git-changes=HEAD",
            "--stdin-filepath",
            "a.md",
            "-",
        ],
        buffer,
    );
    assert_success(&out);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), stdin_only);

    // Both in one run: each keeps its own ranges.
    let out = project.stdin(
        &[
            "--overlaps-git-changes=HEAD",
            "--stdin-filepath",
            "a.md",
            "--stdout",
            "-",
            "b.md",
        ],
        buffer,
    );
    assert_success(&out);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{stdin_only}Base   one.\n\nEdit two.\n")
    );
    assert_eq!(project.read("b.md"), "Base   one.\n\nEdit   two.\n");

    // A buffer with the edited paragraph deleted selects nothing.
    let out = project.stdin(
        &[
            "--overlaps-git-changes=HEAD",
            "--stdin-filepath",
            "a.md",
            "-",
        ],
        "Base   one.\n",
    );
    assert_success(&out);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "Base   one.\n");
}

#[test]
fn generated_diffs_apply_with_git_apply() {
    let project = Project::new();
    project.init_git();
    project.write("a.md", "Top   level.\n");
    project.write("sub/b.md", "Nested   file.\n");
    project.commit();
    project.write("a.md", "Top   level   edit.\n");

    let apply_check = |args: &[&str]| {
        let out = project.run(args);
        let patch = String::from_utf8(out.stdout).unwrap();
        assert!(patch.contains("--- a/"), "{args:?}: {patch}");
        assert!(!patch.contains("/./"), "{args:?}: {patch}");
        let mut child = Command::new("git")
            .current_dir(project.path())
            .args(["apply", "--check", "-"])
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(patch.as_bytes())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert_success(&result);
    };
    apply_check(&["--diff", "."]);
    apply_check(&["--diff", "./sub"]);
    apply_check(&["--diff", "./a.md", "sub/./b.md"]);
    // Git-change filters default to `.`.
    apply_check(&["--diff", "--overlaps-git-changes=uncommitted"]);

    // File names in listings and messages drop `.` components too.
    let out = project.run(&["--dry-run", "."]);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "a.md\nsub/b.md\n");
    let out = project.run(&["--check", "./a.md"]);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Would reformat a.md\n"),
        "{out:?}"
    );
}

/// Runs markfmt with stdin left open (as an editor piping a buffer does) and returns its output
/// once it exits, failing if it waits for stdin's EOF.
fn run_with_open_stdin(project: &Project, args: &[&str]) -> std::process::Output {
    let mut child = project
        .command()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take();
    let start = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > std::time::Duration::from_secs(20) {
            child.kill().unwrap();
            panic!("markfmt {args:?} waited for stdin to close");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    drop(stdin);
    child.wait_with_output().unwrap()
}

#[test]
fn content_independent_errors_do_not_wait_for_stdin() {
    let project = Project::new();
    project.init_git();
    project.write("a.md", "Text.\n");
    project.commit();

    for (args, message) in [
        (
            &["--overlaps-git-changes=HEAD", "-"][..],
            "requires --stdin-filepath",
        ),
        (&["--lines=x", "-"][..], "not a valid positive integer"),
        (
            &[
                "--overlaps-git-changes=HEAD",
                "--stdin-filepath",
                "a.md",
                "-",
                "a.md",
            ][..],
            "cannot format stdin and",
        ),
    ] {
        let out = run_with_open_stdin(&project, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(message),
            "{args:?}: {out:?}"
        );
    }

    project.write(".markfmt.toml", "max_line_width = \"wide\"\n");
    let out = run_with_open_stdin(&project, &["--stdin-filepath", "a.md", "-"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

#[cfg(unix)]
#[test]
fn list_ignored_detects_commit_messages_like_formatting() {
    let project = Project::new();
    project.write(".markfmt.toml", "exclude = [\"msgs/**\"]\n");
    project.write("msgs/COMMIT_EDITMSG", "Subject\n\nBody   text.\n");
    project.write("msgs/plain.txt", "Subject\n\nBody   text.\n");
    std::os::unix::fs::symlink("COMMIT_EDITMSG", project.path().join("msgs/current")).unwrap();

    // Commit messages (named so directly or through a symlink) skip `.markfmt.toml` excludes,
    // in listing as in formatting; `--exclude` still applies.
    let out = project.run(&[
        "--list-ignored",
        "msgs/COMMIT_EDITMSG",
        "msgs/current",
        "msgs/plain.txt",
    ]);
    assert_success(&out);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "msgs/plain.txt\n");
    let out = project.run(&["--stdout", "msgs/current"]);
    assert_success(&out);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "Subject\n\nBody text.\n"
    );
    let out = project.run(&["--list-ignored", "--exclude=msgs/current", "msgs/current"]);
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "msgs/current\n");
}

#[test]
fn summary_counts_only_files_the_git_filter_selects() {
    let project = Project::new();
    project.init_git();
    project.write("a.md", "Committed   one.\n");
    project.write("b.md", "Committed   two.\n");
    project.commit();

    let out = project.run(&["--staged"]);
    assert_success(&out);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "No files to format\n"
    );

    project.write("b.md", "Committed   two.\n\nStaged   three.\n");
    project.git(&["add", "b.md"]);
    let out = project.run(&["--staged"]);
    assert_success(&out);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "1 file reformatted\n"
    );
    assert_eq!(project.read("a.md"), "Committed   one.\n");
}

#[test]
fn ranges_with_an_omitted_side_default_to_head() {
    let project = Project::new();
    project.init_git();
    project.write("a.md", "Base   one.\n");
    project.commit();
    project.write("a.md", "Base   one.\n\nAdded   two.\n");
    project.commit();
    let diff = |mode: &str| {
        let out = project.run(&["--diff", &format!("--overlaps-git-changes={mode}")]);
        assert_success(&out);
        String::from_utf8(out.stdout).unwrap()
    };
    let expected = diff("HEAD~1..HEAD");
    assert!(expected.contains("+Added two."), "{expected}");
    assert_eq!(diff("HEAD~1.."), expected);
    assert_eq!(diff("HEAD~1..."), diff("HEAD~1...HEAD"));
    assert_eq!(diff("..HEAD~1"), diff("HEAD..HEAD~1"));
    for mode in ["..", "...", "a....b"] {
        let out = project.run(&[&format!("--overlaps-git-changes={mode}")]);
        assert_eq!(out.status.code(), Some(2), "{mode}: {out:?}");
    }
}
