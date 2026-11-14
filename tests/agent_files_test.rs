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

use std::fs;
use tempfile::TempDir;

use markfmt::discovery::{discover_targets, InputTarget};

#[test]
fn skill_md_frontmatter_preserved_and_body_formatted() {
    let input = concat!(
        "---\n",
        "name: adaptive-ui\n",
        "description: Instructions for adapting Jetpack Compose UI across extremely long descriptions that must stay untouched inside YAML frontmatter.\n",
        "---\n",
        "\n",
        "\n",
        "# Skill Overview\n",
        "\n",
        "Follow these instructions carefully when updating layout components across multiple screen classes and orientations.\n",
    );
    let opts = common::options(60);
    let expected = concat!(
        "---\n",
        "name: adaptive-ui\n",
        "description: Instructions for adapting Jetpack Compose UI across extremely long descriptions that must stay untouched inside YAML frontmatter.\n",
        "---\n",
        "\n",
        "# Skill Overview\n",
        "\n",
        "Follow these instructions carefully when updating layout\n",
        "components across multiple screen classes and orientations.\n",
    );
    common::assert_format(input, expected, &opts);
}

#[test]
fn standalone_xml_prompt_tags_and_at_imports_allow_inner_reflow() {
    let input = concat!(
        "Always check project rules:\n",
        "@docs/style-guide.md\n",
        "@./AGENTS.md\n",
        "\n",
        "<IMPORTANT>\n",
        "Never skip verification before finishing a task that modifies core formatting modules or test expectations.\n",
        "</IMPORTANT>\n",
        "\n",
        "<details>\n",
        "<summary>Click to see long details</summary>\n",
        "\n",
        "This prose inside details and summary would be swallowed by CommonMark Rule 7 if not treated specially.\n",
        "</details>\n",
    );
    let opts = common::options(60);
    let expected = concat!(
        "Always check project rules:\n",
        "@docs/style-guide.md\n",
        "@./AGENTS.md\n",
        "\n",
        "<IMPORTANT>\n",
        "Never skip verification before finishing a task that\n",
        "modifies core formatting modules or test expectations.\n",
        "</IMPORTANT>\n",
        "\n",
        "<details>\n",
        "<summary>Click to see long details</summary>\n",
        "\n",
        "This prose inside details and summary would be swallowed by\n",
        "CommonMark Rule 7 if not treated specially.\n",
        "</details>\n",
    );
    common::assert_format(input, expected, &opts);
}

#[test]
fn discovers_hidden_agent_directories_while_skipping_vcs_and_respecting_gitignore() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join(".agents/skills/my-skill")).unwrap();
    fs::create_dir_all(root.join(".claude")).unwrap();
    fs::create_dir_all(root.join(".gemini/rules")).unwrap();
    fs::create_dir_all(root.join(".github")).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join("ignored_dir")).unwrap();

    fs::write(root.join("AGENTS.md"), "# Agents\n").unwrap();
    fs::write(
        root.join(".agents/skills/my-skill/SKILL.md"),
        "---\nname: s\n---\n",
    )
    .unwrap();
    fs::write(root.join(".claude/CLAUDE.md"), "# Claude\n").unwrap();
    fs::write(root.join(".gemini/rules/style.md"), "# Style\n").unwrap();
    fs::write(root.join(".github/copilot-instructions.md"), "# GH\n").unwrap();
    fs::write(root.join(".git/COMMIT_EDITMSG.md"), "# Ignore VCS\n").unwrap();
    fs::write(root.join(".gitignore"), "ignored_dir/\n").unwrap();
    fs::write(root.join("ignored_dir/secret.md"), "# Ignored\n").unwrap();
    fs::write(root.join("Component.mdx"), "# Skip MDX\n").unwrap();

    let targets = discover_targets(&[root.to_string_lossy().into_owned()], false)
        .unwrap()
        .targets;
    let names: Vec<String> = targets
        .iter()
        .filter_map(|t| match t {
            InputTarget::File { display_path, .. } => Some(
                display_path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            ),
            InputTarget::Stdin => None,
        })
        .collect();

    assert!(names.contains(&"AGENTS.md".to_string()));
    assert!(names.contains(&".agents/skills/my-skill/SKILL.md".to_string()));
    assert!(names.contains(&".claude/CLAUDE.md".to_string()));
    assert!(names.contains(&".gemini/rules/style.md".to_string()));
    assert!(names.contains(&".github/copilot-instructions.md".to_string()));
    assert!(!names.iter().any(|n| n == ".git" || n.starts_with(".git/")));
    assert!(!names.iter().any(|n| n.contains("ignored_dir")));
    assert!(!names.iter().any(|n| n.ends_with(".mdx")));
}
