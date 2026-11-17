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

use pretty_assertions::assert_eq;
use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_markfmt")
}

#[test]
fn preserves_symlinks_and_deduplicates_canonical_targets() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let agents = root.join("AGENTS.md");
    let claude = root.join("CLAUDE.md");

    let unformatted = "One   two   three   four   five.\n\n\nSecond paragraph.\n";
    fs::write(&agents, unformatted).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&agents, &claude).unwrap();
    #[cfg(windows)]
    if let Err(err) = std::os::windows::fs::symlink_file(&agents, &claude) {
        if err.raw_os_error() == Some(1314) {
            return;
        }
        panic!("failed to create symlink on Windows: {err}");
    }

    let output = Command::new(bin()).arg(root).output().expect("run markfmt");
    assert!(output.status.success(), "{:?}", output);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "1 file reformatted");

    // Crucially: CLAUDE.md MUST remain a symlink!
    assert!(
        fs::symlink_metadata(&claude)
            .unwrap()
            .file_type()
            .is_symlink(),
        "CLAUDE.md symlink was replaced with a regular file!"
    );

    let content = fs::read_to_string(&agents).unwrap();
    assert_eq!(content, "One two three four five.\n\nSecond paragraph.\n");
    assert_eq!(fs::read_to_string(&claude).unwrap(), content);
}

#[test]
fn extensionless_sniffing_and_license_skipping() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Markdown README (extensionless) -> formatted
    let md_readme = root.join("README");
    fs::write(
        &md_readme,
        "#   Project\n\nThis   extensionless   README   should be formatted.\n",
    )
    .unwrap();

    // Plain-text CHANGELOG (no Markdown syntax) -> untouched
    let plain_changelog = root.join("CHANGELOG");
    let plain_body = "2026-01-01  Initial   release.\n            More notes   here.\n";
    fs::write(&plain_changelog, plain_body).unwrap();

    // rST NEWS (extensionless) -> untouched
    let rst_dir = root.join("sub");
    fs::create_dir_all(&rst_dir).unwrap();
    let rst_readme = rst_dir.join("README");
    let rst_body = ".. note::\n\n   This   is   reStructuredText.\n";
    fs::write(&rst_readme, rst_body).unwrap();

    // LICENSE.md -> skipped in directory walk, formatted only when explicitly passed
    let license_md = root.join("LICENSE.md");
    let raw_license = "Copyright   2026   Alice\n\n\nAll   rights   reserved.\n";
    fs::write(&license_md, raw_license).unwrap();

    let walk_out = Command::new(bin()).arg(root).output().unwrap();
    assert!(walk_out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&walk_out.stdout).trim(),
        "1 file reformatted"
    );

    assert_eq!(
        fs::read_to_string(&md_readme).unwrap(),
        "# Project\n\nThis extensionless README should be formatted.\n"
    );
    assert_eq!(fs::read_to_string(&plain_changelog).unwrap(), plain_body);
    assert_eq!(fs::read_to_string(&rst_readme).unwrap(), rst_body);
    assert_eq!(fs::read_to_string(&license_md).unwrap(), raw_license);

    // Explicit file argument formats LICENSE.md
    let explicit_out = Command::new(bin()).arg(&license_md).output().unwrap();
    assert!(explicit_out.status.success());
    assert_eq!(
        fs::read_to_string(&license_md).unwrap(),
        "Copyright 2026 Alice\n\nAll rights reserved.\n"
    );
}

#[test]
fn editorconfig_specificity_overrides_and_line_endings() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::write(
        root.join(".editorconfig"),
        concat!(
            "root = true\n",
            "\n",
            "[*]\n",
            "max_line_length = 120\n",
            "indent_size = 4\n",
            "end_of_line = lf\n",
            "\n",
            "[*.{md,markdown}]\n",
            "max_line_length = 40\n",
            "\n",
            "[docs/*.md]\n",
            "max_line_length = 30\n",
            "end_of_line = crlf\n",
        ),
    )
    .unwrap();

    // `docs/guide.md` matches `[docs/*.md]` (width 30, CRLF). `notes.mkd` matches no
    // Markdown-specific section, so the catch-all `[*]` width of 120 (meant for code) must not
    // apply to it; it keeps the default width of 72.
    let mkd_file = root.join("notes.mkd");
    let long_para = "Word1 word2 word3 word4 word5 word6 word7 word8 word9 word10 word11 word12 word13 word14 word15 word16.\n";
    fs::write(&mkd_file, long_para).unwrap();

    fs::create_dir_all(root.join("docs")).unwrap();
    let doc_md = root.join("docs/guide.md");
    fs::write(&doc_md, long_para).unwrap();

    let out = Command::new(bin()).arg(root).output().unwrap();
    assert!(out.status.success());

    // `.mkd` didn't match `[*.{md,markdown}]` and MUST ignore `[*]` 120, keeping default 72!
    let formatted_mkd = fs::read_to_string(&mkd_file).unwrap();
    assert!(
        formatted_mkd.lines().all(|l| l.len() <= 72) && formatted_mkd.lines().count() >= 2,
        "notes.mkd incorrectly picked up [*] max_line_length=120:\n{formatted_mkd}"
    );

    // `docs/guide.md` matched `[docs/*.md]` -> width 30, CRLF
    let formatted_doc = fs::read_to_string(&doc_md).unwrap();
    assert!(
        formatted_doc.contains("\r\n"),
        "expected CRLF from .editorconfig"
    );
    for line in formatted_doc.lines() {
        assert!(line.len() <= 30, "expected width 30, got {line:?}");
    }

    // CLI `--max-line-width` wins over `.editorconfig`, and `--no-editorconfig` ignores it.
    let cli_override = Command::new(bin())
        .args(["--max-line-width=100", "--no-editorconfig", "--diff"])
        .arg(&doc_md)
        .output()
        .unwrap();
    assert!(cli_override.status.success());
    let diff_text = String::from_utf8_lossy(&cli_override.stdout);
    assert!(!diff_text.is_empty());
}

#[test]
fn check_diff_dry_run_stdin_and_argfile() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let f = root.join("test.md");
    let raw = "Hello   world.\n";
    fs::write(&f, raw).unwrap();

    // --check exits 1 and doesn't modify
    let check_fail = Command::new(bin()).arg("--check").arg(&f).output().unwrap();
    assert_eq!(check_fail.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&f).unwrap(), raw);

    // --diff prints unified diff and doesn't modify
    let diff_out = Command::new(bin()).arg("--diff").arg(&f).output().unwrap();
    assert!(diff_out.status.success());
    let diff_s = String::from_utf8_lossy(&diff_out.stdout);
    assert!(diff_s.contains("-Hello   world."));
    assert!(diff_s.contains("+Hello world."));
    assert_eq!(fs::read_to_string(&f).unwrap(), raw);

    // --dry-run prints file path, exits 0, and doesn't modify
    let dry_out = Command::new(bin())
        .arg("--dry-run")
        .arg(&f)
        .output()
        .unwrap();
    assert!(dry_out.status.success());
    assert!(String::from_utf8_lossy(&dry_out.stdout).contains("test.md"));
    assert_eq!(fs::read_to_string(&f).unwrap(), raw);

    // @argfile formats file, then --check exits 0
    let argfile = root.join("files.txt");
    fs::write(&argfile, format!("{}\n", f.display())).unwrap();
    let run_arg = Command::new(bin())
        .arg(format!("@{}", argfile.display()))
        .output()
        .unwrap();
    assert!(run_arg.status.success());
    assert_eq!(fs::read_to_string(&f).unwrap(), "Hello world.\n");

    let check_ok = Command::new(bin()).arg("--check").arg(&f).output().unwrap();
    assert!(check_ok.status.success());

    // stdin `-` with `--stdin-filepath`
    let mut child = Command::new(bin())
        .args([
            "--stdin-filepath",
            "virtual.md.html",
            "--max-line-width",
            "35",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"!!! note\n    Words that wrap nicely at 35 cols.\n")
        .unwrap();
    let stdin_res = child.wait_with_output().unwrap();
    assert!(stdin_res.status.success());
    assert_eq!(
        String::from_utf8_lossy(&stdin_res.stdout),
        "!!! note\n    Words that wrap nicely at 35\n    cols.\n"
    );
}

#[test]
fn partial_formatting_lines_and_overlaps_git_changes() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let file = root.join("doc.md");
    let v1 = concat!(
        "First   unformatted   paragraph.\n",
        "\n\n\n",
        "Second   unformatted   paragraph.\n",
    );
    fs::write(&file, v1).unwrap();

    // `--lines 1` only touches First paragraph, preserving `\n\n\n` and Second paragraph verbatim!
    let lines_out = Command::new(bin())
        .args(["--lines", "1"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(lines_out.status.success());
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "First unformatted paragraph.\n\n\n\nSecond   unformatted   paragraph.\n"
    );

    // Test `--overlaps-git-changes=staged` and `HEAD` in a temporary git repository.
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let git_dir = root.join("repo");
    fs::create_dir_all(&git_dir).unwrap();
    run_git(&git_dir, &["init"]);
    run_git(&git_dir, &["config", "core.autocrlf", "false"]);
    run_git(&git_dir, &["config", "user.email", "test@example.com"]);
    run_git(&git_dir, &["config", "user.name", "Test User"]);

    let git_doc = git_dir.join("notes.md");
    fs::write(
        &git_doc,
        "Untouched   legacy   paragraph.\n\nAnother   legacy   line.\n",
    )
    .unwrap();
    run_git(&git_dir, &["add", "notes.md"]);
    run_git(&git_dir, &["commit", "-m", "initial"]);

    // Modify ONLY paragraph 2 and stage it; paragraph 1 stays unformatted legacy text.
    fs::write(
        &git_doc,
        "Untouched   legacy   paragraph.\n\nModified   second   line   by   author.\n",
    )
    .unwrap();
    run_git(&git_dir, &["add", "notes.md"]);

    let staged_out = Command::new(bin())
        .arg("--overlaps-git-changes=staged")
        .arg(&git_dir)
        .output()
        .unwrap();
    assert!(staged_out.status.success(), "{:?}", staged_out);
    assert_eq!(
        fs::read_to_string(&git_doc).unwrap(),
        "Untouched   legacy   paragraph.\n\nModified second line by author.\n"
    );

    // Commit another edit to paragraph 2 and test `--overlaps-git-changes=HEAD`.
    fs::write(
        &git_doc,
        "Untouched   legacy   paragraph.\n\nCommitted   third   edit   here.\n",
    )
    .unwrap();
    run_git(&git_dir, &["add", "notes.md"]);
    run_git(&git_dir, &["commit", "-m", "second"]);

    let head_out = Command::new(bin())
        .arg("--overlaps-git-changes=HEAD")
        .arg(&git_dir)
        .output()
        .unwrap();
    assert!(head_out.status.success(), "{:?}", head_out);
    assert_eq!(
        fs::read_to_string(&git_doc).unwrap(),
        "Untouched   legacy   paragraph.\n\nCommitted third edit here.\n"
    );
}

fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git command failed to spawn");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn version_baked_into_version_and_help_flags() {
    let ver = markfmt::version();
    assert_eq!(ver, env!("CARGO_PKG_VERSION"));
    assert!(!ver.is_empty());

    let expected_header = format!("markfmt {ver}");
    for flag in ["--version", "-V", "--help", "-h", "-help"] {
        let out = Command::new(bin())
            .arg(flag)
            .output()
            .unwrap_or_else(|e| panic!("failed to run `{flag}`: {e}"));
        assert!(out.status.success(), "`{flag}` failed: {:?}", out);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let first_line = stdout.lines().next().unwrap_or("");
        assert_eq!(
            first_line, expected_header,
            "unexpected first line for `{flag}`:\n{stdout}"
        );
    }
}

#[test]
fn completions_and_man_page_for_packagers() {
    // Each shell's script names the binary and offers its long options.
    for shell in ["bash", "elvish", "fish", "powershell", "zsh"] {
        let out = Command::new(bin())
            .args(["--completions", shell])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "`--completions {shell}` failed: {out:?}"
        );
        let script = String::from_utf8_lossy(&out.stdout);
        assert!(script.contains("markfmt"), "{shell}: {script}");
        assert!(script.contains("max-line-width"), "{shell}: {script}");
        // Unescaped `"` inside enum help breaks Fish/Zsh quoting, and `:` in value names breaks
        // Zsh `_arguments` action specs.
        if matches!(shell, "fish" | "zsh") {
            assert!(!script.contains("(\"semantic"), "{shell}: {script}");
        }
        if shell == "zsh" {
            assert!(!script.contains(":START:END:"), "{script}");
            assert!(script.contains(":_files"), "{script}");
        }
        if shell == "fish" {
            assert!(
                script.contains("-l max-line-width") && script.contains(" -r -f"),
                "{script}"
            );
        }
    }

    let out = Command::new(bin()).arg("--man").output().unwrap();
    assert!(out.status.success(), "`--man` failed: {out:?}");
    let man = String::from_utf8_lossy(&out.stdout);
    let title = format!(".TH markfmt 1  \"markfmt {}\"", env!("CARGO_PKG_VERSION"));
    assert!(man.contains(&title), "{man}");
    assert!(man.contains(r"\-\-max\-line\-width"), "{man}");

    // Neither mixes with formatting work.
    for args in [
        &["--man", "README.md"][..],
        &["--completions", "zsh", "--check"],
    ] {
        let out = Command::new(bin()).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
    }
}

#[test]
fn overlaps_git_changes_matches_non_ascii_paths() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let git_dir = tmp.path().join("repo");
    fs::create_dir_all(&git_dir).unwrap();
    run_git(&git_dir, &["init"]);
    run_git(&git_dir, &["config", "core.autocrlf", "false"]);
    // Git's default; set explicitly so the test does not depend on the user's global config.
    run_git(&git_dir, &["config", "core.quotePath", "true"]);
    run_git(&git_dir, &["config", "user.email", "test@example.com"]);
    run_git(&git_dir, &["config", "user.name", "Test User"]);

    let doc = git_dir.join("café notes.md");
    fs::write(&doc, "Staged   unformatted   paragraph.\n").unwrap();
    run_git(&git_dir, &["add", "."]);

    let out = Command::new(bin())
        .arg("--overlaps-git-changes=staged")
        .arg(&git_dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "{:?}", out);
    assert_eq!(
        fs::read_to_string(&doc).unwrap(),
        "Staged unformatted paragraph.\n"
    );
}

#[test]
fn bad_file_is_reported_without_aborting_other_files() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    // Sorted discovery order puts the unreadable file first.
    fs::write(root.join("a_invalid.md"), b"Not \xff valid   UTF-8.\n").unwrap();
    fs::write(root.join("b_good.md"), "Needs   formatting.\n").unwrap();

    let out = Command::new(bin()).arg(root).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("a_invalid.md"), "{stderr}");
    assert_eq!(
        fs::read_to_string(root.join("b_good.md")).unwrap(),
        "Needs formatting.\n"
    );
}

#[cfg(unix)]
#[test]
fn writes_preserve_permissions_and_leave_no_temp_files() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let doc = tmp.path().join("doc.md");
    fs::write(&doc, "Needs   formatting.\n").unwrap();
    fs::set_permissions(&doc, fs::Permissions::from_mode(0o640)).unwrap();

    let out = Command::new(bin()).arg(&doc).output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fs::read_to_string(&doc).unwrap(), "Needs formatting.\n");
    assert_eq!(
        fs::metadata(&doc).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let entries: Vec<_> = fs::read_dir(tmp.path()).unwrap().collect();
    assert_eq!(entries.len(), 1, "temporary file left behind: {entries:?}");
}

#[test]
fn config_file_settings_excludes_and_precedence() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let long = "Alpha beta gamma delta. Epsilon zeta eta theta iota kappa.\n";
    fs::write(
        root.join(".markfmt.toml"),
        "max_line_width = 30\nwrap = \"sentence\"\nexclude = [\"vendor/\", \"SKIP.md\"]\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("vendor")).unwrap();
    fs::create_dir_all(root.join("sub")).unwrap();
    for name in ["a.md", "SKIP.md", "vendor/v.md", "sub/s.md"] {
        fs::write(root.join(name), long).unwrap();
    }
    // The nearest config wins (no merging): `sub/` uses only its own settings.
    fs::write(root.join("sub/.markfmt.toml"), "max_line_width = 40\n").unwrap();

    let out = Command::new(bin()).arg(root).output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        fs::read_to_string(root.join("a.md")).unwrap(),
        "Alpha beta gamma delta.\nEpsilon zeta eta theta iota\nkappa.\n"
    );
    assert_eq!(fs::read_to_string(root.join("SKIP.md")).unwrap(), long);
    assert_eq!(fs::read_to_string(root.join("vendor/v.md")).unwrap(), long);
    assert_eq!(
        fs::read_to_string(root.join("sub/s.md")).unwrap(),
        "Alpha beta gamma delta. Epsilon zeta eta\ntheta iota kappa.\n"
    );

    // Excludes also apply to explicitly named files (as passed by pre-commit).
    let out = Command::new(bin())
        .arg(root.join("SKIP.md"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(fs::read_to_string(root.join("SKIP.md")).unwrap(), long);

    // Command-line flags override the config; --no-config ignores it.
    fs::write(root.join("a.md"), long).unwrap();
    let out = Command::new(bin())
        .args(["--wrap", "always"])
        .arg(root.join("a.md"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        fs::read_to_string(root.join("a.md")).unwrap(),
        "Alpha beta gamma delta.\nEpsilon zeta eta theta iota\nkappa.\n"
    );
    let out = Command::new(bin())
        .arg("--no-config")
        .arg(root.join("SKIP.md"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        fs::read_to_string(root.join("SKIP.md")).unwrap(),
        "Alpha beta gamma delta. Epsilon zeta eta theta iota kappa.\n"
    );
}

#[test]
fn exclude_flag_and_invalid_config() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(root.join("keep.md"), "Needs   formatting.\n").unwrap();
    fs::write(root.join("gen.md"), "Needs   formatting.\n").unwrap();

    let out = Command::new(bin())
        .current_dir(root)
        .args(["--exclude", "gen.md", "."])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        fs::read_to_string(root.join("keep.md")).unwrap(),
        "Needs formatting.\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("gen.md")).unwrap(),
        "Needs   formatting.\n"
    );

    fs::write(root.join(".markfmt.toml"), "max_line_widht = 80\n").unwrap();
    let out = Command::new(bin()).arg(root).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("max_line_widht"), "{stderr}");
}

#[test]
fn overlaps_git_changes_uncommitted_includes_unstaged_and_untracked() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "core.autocrlf", "false"]);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "user.name", "Test User"]);

    let tracked = repo.join("tracked.md");
    fs::write(&tracked, "Old   first   paragraph.\n\nOld   second.\n").unwrap();
    fs::write(repo.join("committed.md"), "Committed   and   untouched.\n").unwrap();
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "initial"]);

    // Unstaged edit to paragraph 2 only, plus a brand-new untracked file.
    fs::write(&tracked, "Old   first   paragraph.\n\nNew   second.\n").unwrap();
    fs::write(repo.join("new.md"), "Brand   new   file.\n").unwrap();

    let out = Command::new(bin())
        .arg("--overlaps-git-changes=uncommitted")
        .arg(&repo)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        fs::read_to_string(&tracked).unwrap(),
        "Old   first   paragraph.\n\nNew second.\n"
    );
    assert_eq!(
        fs::read_to_string(repo.join("new.md")).unwrap(),
        "Brand new file.\n"
    );
    assert_eq!(
        fs::read_to_string(repo.join("committed.md")).unwrap(),
        "Committed   and   untouched.\n"
    );
}

#[test]
fn summaries_check_hint_and_quiet() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(root.join("a.md"), "Needs   formatting.\n").unwrap();
    fs::write(root.join("b.md"), "Already formatted.\n").unwrap();

    let check = Command::new(bin())
        .arg("--check")
        .arg(root)
        .output()
        .unwrap();
    assert_eq!(check.status.code(), Some(1), "{check:?}");
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(stderr.contains("Would reformat "), "{stderr}");
    assert!(
        stderr.contains("1 file would be reformatted, 1 file already formatted\n"),
        "{stderr}"
    );
    assert!(stderr.contains("Run markfmt without --check"), "{stderr}");
    assert!(check.stdout.is_empty());

    let quiet = Command::new(bin())
        .args(["--check", "--quiet"])
        .arg(root)
        .output()
        .unwrap();
    assert_eq!(quiet.status.code(), Some(1));
    assert!(
        quiet.stdout.is_empty() && quiet.stderr.is_empty(),
        "{quiet:?}"
    );

    let format = Command::new(bin()).arg(root).output().unwrap();
    assert!(format.status.success());
    assert_eq!(
        String::from_utf8_lossy(&format.stdout),
        "1 file reformatted, 1 file unchanged\n"
    );

    let clean = Command::new(bin())
        .arg("--check")
        .arg(root)
        .output()
        .unwrap();
    assert!(clean.status.success());
    assert_eq!(
        String::from_utf8_lossy(&clean.stderr),
        "2 files already formatted\n"
    );
}

#[test]
fn diff_is_plain_unless_color_is_forced() {
    let tmp = TempDir::new().unwrap();
    let doc = tmp.path().join("doc.md");
    fs::write(&doc, "One   two three.\n").unwrap();
    let run = |args: &[&str], no_color: bool| {
        let mut cmd = Command::new(bin());
        cmd.current_dir(tmp.path()).args(args).arg("doc.md");
        if no_color {
            cmd.env("NO_COLOR", "1");
        } else {
            cmd.env_remove("NO_COLOR");
        }
        String::from_utf8(cmd.output().unwrap().stdout).unwrap()
    };

    let plain = "--- a/doc.md\n+++ b/doc.md\n@@ -1 +1 @@\n-One   two three.\n+One two three.\n";
    // Piped output (as in tests) is a standard unified diff.
    assert_eq!(run(&["--diff"], false), plain);
    assert_eq!(run(&["--diff", "--color", "never"], false), plain);
    assert_eq!(run(&["--diff", "--color", "auto"], true), plain);

    let colored = run(&["--diff", "--color", "always"], true);
    assert!(colored.contains('\u{1b}'), "{colored:?}");
    let stripped: String = {
        let mut out = String::new();
        let mut chars = colored.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for c in chars.by_ref() {
                    if c == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    };
    assert_eq!(stripped, plain);
    // Only whitespace changed, so no word is highlighted (inverse video is SGR 7).
    assert!(!colored.contains("\u{1b}[7m"), "{colored:?}");
}

#[cfg(unix)]
#[test]
fn install_script_verifies_checksum_and_installs_binary() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let ver = markfmt::version();
    let tag = format!("v{ver}");
    let target = "x86_64-unknown-linux-musl";
    let stage_name = format!("markfmt-{tag}-{target}");
    let archive_name = format!("{stage_name}.tar.gz");

    let stage_dir = root.join("stage").join(&stage_name);
    fs::create_dir_all(&stage_dir).unwrap();
    fs::copy(bin(), stage_dir.join("markfmt")).unwrap();

    let release_dir = root.join("releases").join(&tag);
    fs::create_dir_all(&release_dir).unwrap();
    let archive_path = release_dir.join(&archive_name);
    let tar_status = Command::new("tar")
        .arg("-C")
        .arg(root.join("stage"))
        .args(["-czf"])
        .arg(&archive_path)
        .arg(&stage_name)
        .status()
        .unwrap();
    assert!(tar_status.success(), "tar failed: {tar_status:?}");

    let sha_out = Command::new("/bin/sh")
        .args([
            "-c",
            "if command -v sha256sum >/dev/null 2>&1; then sha256sum \"$1\"; else shasum -a 256 \"$1\"; fi",
            "_",
        ])
        .arg(&archive_path)
        .output()
        .unwrap();
    assert!(sha_out.status.success(), "sha256 failed: {sha_out:?}");
    let digest = String::from_utf8_lossy(&sha_out.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    fs::write(
        release_dir.join("SHA256SUMS.txt"),
        format!("{digest}  {archive_name}\n"),
    )
    .unwrap();

    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/install.sh");
    let stamped_script = root.join("install.sh");
    let stamped = fs::read_to_string(&script)
        .unwrap()
        .replace("__MARKFMT_RELEASE_TAG__", &tag);
    fs::write(&stamped_script, stamped).unwrap();

    let base_url = format!("file://{}", root.join("releases").display());
    let install_dir = root.join("bin");
    let out = Command::new("/bin/sh")
        .arg(&stamped_script)
        .env("MARKFMT_BASE_URL", &base_url)
        .env("MARKFMT_TARGET", target)
        .env("MARKFMT_INSTALL_DIR", &install_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "install.sh failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("export PATH="),
        "expected PATH hint in stdout: {stdout}"
    );

    let installed = install_dir.join("markfmt");
    let ver_out = Command::new(&installed).arg("--version").output().unwrap();
    assert!(ver_out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&ver_out.stdout).trim(),
        format!("markfmt {ver}")
    );

    // Checksum mismatch must fail before creating or overwriting the target binary.
    fs::write(
        release_dir.join("SHA256SUMS.txt"),
        format!("{}  {archive_name}\n", "0".repeat(64)),
    )
    .unwrap();
    let bad_dir = root.join("bad-bin");
    let bad_out = Command::new("/bin/sh")
        .arg(&script)
        .args(["--version", ver, "--dir"])
        .arg(&bad_dir)
        .env("MARKFMT_BASE_URL", &base_url)
        .env("MARKFMT_TARGET", target)
        .output()
        .unwrap();
    assert!(!bad_out.status.success(), "expected checksum failure");
    let stderr = String::from_utf8_lossy(&bad_out.stderr);
    assert!(
        stderr.contains("Checksum verification failed"),
        "unexpected stderr: {stderr}"
    );
    assert!(!bad_dir.join("markfmt").exists());
}
