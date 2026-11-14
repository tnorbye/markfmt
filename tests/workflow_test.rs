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
use std::process::Stdio;

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
    let text = String::from_utf8(output.stdout).unwrap();
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
    let text = String::from_utf8(output.stdout).unwrap();
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
