// Copyright 2022 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::path::Path;

use indoc::indoc;

use crate::common::TestEnvironment;

fn create_commit(
    test_env: &TestEnvironment,
    repo_path: &Path,
    name: &str,
    parents: &[&str],
    files: &[(&str, &str)],
) {
    if parents.is_empty() {
        test_env.jj_cmd_ok(repo_path, &["new", "root()", "-m", name]);
    } else {
        let mut args = vec!["new", "-m", name];
        args.extend(parents);
        test_env.jj_cmd_ok(repo_path, &args);
    }
    for (name, content) in files {
        std::fs::write(repo_path.join(name), content).unwrap();
    }
    test_env.jj_cmd_ok(repo_path, &["bookmark", "create", name]);
}

fn get_log_output(test_env: &TestEnvironment, repo_path: &Path) -> String {
    test_env.jj_cmd_success(repo_path, &["log", "-T", "bookmarkes"])
}

#[test]
fn test_resolution() {
    let mut test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "base", &[], &[("file", "base\n")]);
    create_commit(&test_env, &repo_path, "a", &["base"], &[("file", "a\n")]);
    create_commit(&test_env, &repo_path, "b", &["base"], &[("file", "b\n")]);
    create_commit(&test_env, &repo_path, "conflict", &["a", "b"], &[]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @    conflict
    ├─╮
    │ ○  b
    ○ │  a
    ├─╯
    ○  base
    ◆
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]), 
    @r###"
    file    2-sided conflict
    "###);
    insta::assert_snapshot!(
    std::fs::read_to_string(repo_path.join("file")).unwrap()
        , @r###"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -base
    +a
    +++++++ Contents of side #2
    b
    >>>>>>> Conflict 1 of 1 ends
    "###);

    let editor_script = test_env.set_up_fake_editor();
    // Check that output file starts out empty and resolve the conflict
    std::fs::write(
        &editor_script,
        ["dump editor0", "write\nresolution\n"].join("\0"),
    )
    .unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["resolve"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Resolving conflicts in: file
    Working copy now at: vruxwmqv e069f073 conflict | conflict
    Parent commit      : zsuskuln aa493daf a | a
    Parent commit      : royxmykx db6a4daf b | b
    Added 0 files, modified 1 files, removed 0 files
    "###);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor0")).unwrap(), @r###"
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]), 
    @r###"
    diff --git a/file b/file
    index 0000000000..88425ec521 100644
    --- a/file
    +++ b/file
    @@ -1,7 +1,1 @@
    -<<<<<<< Conflict 1 of 1
    -%%%%%%% Changes from base to side #1
    --base
    -+a
    -+++++++ Contents of side #2
    -b
    ->>>>>>> Conflict 1 of 1 ends
    +resolution
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_cli_error(&repo_path, &["resolve", "--list"]), 
    @r###"
    Error: No conflicts found at this revision
    "###);

    // Try again with --tool=<name>
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    std::fs::write(&editor_script, "write\nresolution\n").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &repo_path,
        &[
            "resolve",
            "--config-toml=ui.merge-editor='false'",
            "--tool=fake-editor",
        ],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Resolving conflicts in: file
    Working copy now at: vruxwmqv 1a70c7c6 conflict | conflict
    Parent commit      : zsuskuln aa493daf a | a
    Parent commit      : royxmykx db6a4daf b | b
    Added 0 files, modified 1 files, removed 0 files
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]),
    @r###"
    diff --git a/file b/file
    index 0000000000..88425ec521 100644
    --- a/file
    +++ b/file
    @@ -1,7 +1,1 @@
    -<<<<<<< Conflict 1 of 1
    -%%%%%%% Changes from base to side #1
    --base
    -+a
    -+++++++ Contents of side #2
    -b
    ->>>>>>> Conflict 1 of 1 ends
    +resolution
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_cli_error(&repo_path, &["resolve", "--list"]),
    @r###"
    Error: No conflicts found at this revision
    "###);

    // Check that the output file starts with conflict markers if
    // `merge-tool-edits-conflict-markers=true`
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]),
    @"");
    std::fs::write(
        &editor_script,
        ["dump editor1", "write\nresolution\n"].join("\0"),
    )
    .unwrap();
    test_env.jj_cmd_ok(
        &repo_path,
        &[
            "resolve",
            "--config-toml",
            "merge-tools.fake-editor.merge-tool-edits-conflict-markers=true",
        ],
    );
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r###"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -base
    +a
    +++++++ Contents of side #2
    b
    >>>>>>> Conflict 1 of 1 ends
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]), 
    @r###"
    diff --git a/file b/file
    index 0000000000..88425ec521 100644
    --- a/file
    +++ b/file
    @@ -1,7 +1,1 @@
    -<<<<<<< Conflict 1 of 1
    -%%%%%%% Changes from base to side #1
    --base
    -+a
    -+++++++ Contents of side #2
    -b
    ->>>>>>> Conflict 1 of 1 ends
    +resolution
    "###);

    // Check that if merge tool leaves conflict markers in output file and
    // `merge-tool-edits-conflict-markers=true`, these markers are properly parsed.
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]), 
    @"");
    std::fs::write(
        &editor_script,
        [
            "dump editor2",
            indoc! {"
                write
                <<<<<<<
                %%%%%%%
                -some
                +fake
                +++++++
                conflict
                >>>>>>>
            "},
        ]
        .join("\0"),
    )
    .unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &repo_path,
        &[
            "resolve",
            "--config-toml",
            "merge-tools.fake-editor.merge-tool-edits-conflict-markers=true",
        ],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Resolving conflicts in: file
    New conflicts appeared in these commits:
      vruxwmqv 7699b9c3 conflict | (conflict) conflict
    To resolve the conflicts, start by updating to it:
      jj new vruxwmqvtpmx
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you may want to inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    Working copy now at: vruxwmqv 7699b9c3 conflict | (conflict) conflict
    Parent commit      : zsuskuln aa493daf a | a
    Parent commit      : royxmykx db6a4daf b | b
    Added 0 files, modified 1 files, removed 0 files
    There are unresolved conflicts at these paths:
    file    2-sided conflict
    "###);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor2")).unwrap(), @r###"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -base
    +a
    +++++++ Contents of side #2
    b
    >>>>>>> Conflict 1 of 1 ends
    "###);
    // Note the "Modified" below
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]), 
    @r###"
    diff --git a/file b/file
    --- a/file
    +++ b/file
    @@ -1,7 +1,7 @@
     <<<<<<< Conflict 1 of 1
     %%%%%%% Changes from base to side #1
    --base
    -+a
    +-some
    ++fake
     +++++++ Contents of side #2
    -b
    +conflict
     >>>>>>> Conflict 1 of 1 ends
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]), 
    @r###"
    file    2-sided conflict
    "###);

    // Check that if merge tool leaves conflict markers in output file but
    // `merge-tool-edits-conflict-markers=false` or is not specified,
    // `jj` considers the conflict resolved.
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]), 
    @"");
    std::fs::write(
        &editor_script,
        [
            "dump editor3",
            indoc! {"
                write
                <<<<<<<
                %%%%%%%
                -some
                +fake
                +++++++
                conflict
                >>>>>>>
            "},
        ]
        .join("\0"),
    )
    .unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["resolve"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Resolving conflicts in: file
    Working copy now at: vruxwmqv 3166dfd2 conflict | conflict
    Parent commit      : zsuskuln aa493daf a | a
    Parent commit      : royxmykx db6a4daf b | b
    Added 0 files, modified 1 files, removed 0 files
    "###);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor3")).unwrap(), @r###"
    "###);
    // Note the "Resolved" below
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]), 
    @r###"
    diff --git a/file b/file
    index 0000000000..0610716cc1 100644
    --- a/file
    +++ b/file
    @@ -1,7 +1,7 @@
    -<<<<<<< Conflict 1 of 1
    -%%%%%%% Changes from base to side #1
    --base
    -+a
    -+++++++ Contents of side #2
    -b
    ->>>>>>> Conflict 1 of 1 ends
    +<<<<<<<
    +%%%%%%%
    +-some
    ++fake
    ++++++++
    +conflict
    +>>>>>>>
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_cli_error(&repo_path, &["resolve", "--list"]), 
    @r###"
    Error: No conflicts found at this revision
    "###);

    // TODO: Check that running `jj new` and then `jj resolve -r conflict` works
    // correctly.
}

fn check_resolve_produces_input_file(
    test_env: &mut TestEnvironment,
    repo_path: &Path,
    filename: &str,
    role: &str,
    expected_content: &str,
) {
    let editor_script = test_env.set_up_fake_editor();
    std::fs::write(editor_script, format!("expect\n{expected_content}")).unwrap();

    let merge_arg_config = format!(r#"merge-tools.fake-editor.merge-args = ["${role}"]"#);
    // This error means that fake-editor exited successfully but did not modify the
    // output file.
    // We cannot use `insta::assert_snapshot!` here after insta 1.22 due to
    // https://github.com/mitsuhiko/insta/commit/745b45b. Hopefully, this will again become possible
    // in the future. See also https://github.com/mitsuhiko/insta/issues/313.
    assert_eq!(
        test_env.jj_cmd_failure(
            repo_path,
            &["resolve", "--config-toml", &merge_arg_config, filename]
        ),
        format!(
            "Resolving conflicts in: {filename}\nError: Failed to resolve conflicts\nCaused by: \
             The output file is either unchanged or empty after the editor quit (run with --debug \
             to see the exact invocation).\n"
        )
    );
}

#[test]
fn test_normal_conflict_input_files() {
    let mut test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "base", &[], &[("file", "base\n")]);
    create_commit(&test_env, &repo_path, "a", &["base"], &[("file", "a\n")]);
    create_commit(&test_env, &repo_path, "b", &["base"], &[("file", "b\n")]);
    create_commit(&test_env, &repo_path, "conflict", &["a", "b"], &[]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @    conflict
    ├─╮
    │ ○  b
    ○ │  a
    ├─╯
    ○  base
    ◆
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]), 
    @r###"
    file    2-sided conflict
    "###);
    insta::assert_snapshot!(
    std::fs::read_to_string(repo_path.join("file")).unwrap()
        , @r###"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -base
    +a
    +++++++ Contents of side #2
    b
    >>>>>>> Conflict 1 of 1 ends
    "###);

    check_resolve_produces_input_file(&mut test_env, &repo_path, "file", "base", "base\n");
    check_resolve_produces_input_file(&mut test_env, &repo_path, "file", "left", "a\n");
    check_resolve_produces_input_file(&mut test_env, &repo_path, "file", "right", "b\n");
}

#[test]
fn test_baseless_conflict_input_files() {
    let mut test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "base", &[], &[]);
    create_commit(&test_env, &repo_path, "a", &["base"], &[("file", "a\n")]);
    create_commit(&test_env, &repo_path, "b", &["base"], &[("file", "b\n")]);
    create_commit(&test_env, &repo_path, "conflict", &["a", "b"], &[]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @    conflict
    ├─╮
    │ ○  b
    ○ │  a
    ├─╯
    ○  base
    ◆
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]), 
    @r###"
    file    2-sided conflict
    "###);
    insta::assert_snapshot!(
    std::fs::read_to_string(repo_path.join("file")).unwrap()
        , @r###"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    +a
    +++++++ Contents of side #2
    b
    >>>>>>> Conflict 1 of 1 ends
    "###);

    check_resolve_produces_input_file(&mut test_env, &repo_path, "file", "base", "");
    check_resolve_produces_input_file(&mut test_env, &repo_path, "file", "left", "a\n");
    check_resolve_produces_input_file(&mut test_env, &repo_path, "file", "right", "b\n");
}

#[test]
fn test_too_many_parents() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "base", &[], &[("file", "base\n")]);
    create_commit(&test_env, &repo_path, "a", &["base"], &[("file", "a\n")]);
    create_commit(&test_env, &repo_path, "b", &["base"], &[("file", "b\n")]);
    create_commit(&test_env, &repo_path, "c", &["base"], &[("file", "c\n")]);
    create_commit(&test_env, &repo_path, "conflict", &["a", "b", "c"], &[]);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]), 
    @r###"
    file    3-sided conflict
    "###);
    // Test warning color
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list", "--color=always"]), 
    @r###"
    file    [38;5;1m3-sided[38;5;3m conflict[39m
    "###);

    let error = test_env.jj_cmd_failure(&repo_path, &["resolve"]);
    insta::assert_snapshot!(error, @r###"
    Hint: Using default editor ':builtin'; run `jj config set --user ui.merge-editor :builtin` to disable this message.
    Resolving conflicts in: file
    Error: Failed to resolve conflicts
    Caused by: The conflict at "file" has 3 sides. At most 2 sides are supported.
    "###);
}

#[test]
fn test_simplify_conflict_sides() {
    let mut test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    // Creates a 4-sided conflict, with fileA and fileB having different conflicts:
    // fileA: A - B + C - B + B - B + B
    // fileB: A - A + A - A + B - C + D
    create_commit(
        &test_env,
        &repo_path,
        "base",
        &[],
        &[("fileA", "base\n"), ("fileB", "base\n")],
    );
    create_commit(&test_env, &repo_path, "a1", &["base"], &[("fileA", "1\n")]);
    create_commit(&test_env, &repo_path, "a2", &["base"], &[("fileA", "2\n")]);
    create_commit(&test_env, &repo_path, "b1", &["base"], &[("fileB", "1\n")]);
    create_commit(&test_env, &repo_path, "b2", &["base"], &[("fileB", "2\n")]);
    create_commit(&test_env, &repo_path, "conflictA", &["a1", "a2"], &[]);
    create_commit(&test_env, &repo_path, "conflictB", &["b1", "b2"], &[]);
    create_commit(
        &test_env,
        &repo_path,
        "conflict",
        &["conflictA", "conflictB"],
        &[],
    );

    // Even though the tree-level conflict is a 4-sided conflict, each file is
    // materialized as a 2-sided conflict.
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["debug", "tree"]),
    @r###"
    fileA: Ok(Conflicted([Some(File { id: FileId("d00491fd7e5bb6fa28c517a0bb32b8b506539d4d"), executable: false }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false }), Some(File { id: FileId("0cfbf08886fca9a91cb753ec8734c84fcbe52c9f"), executable: false }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false })]))
    fileB: Ok(Conflicted([Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false }), Some(File { id: FileId("d00491fd7e5bb6fa28c517a0bb32b8b506539d4d"), executable: false }), Some(File { id: FileId("df967b96a579e45a18b8251732d16804b2e56a55"), executable: false }), Some(File { id: FileId("0cfbf08886fca9a91cb753ec8734c84fcbe52c9f"), executable: false })]))
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]),
    @r###"
    fileA    2-sided conflict
    fileB    2-sided conflict
    "###);
    insta::assert_snapshot!(
    std::fs::read_to_string(repo_path.join("fileA")).unwrap(), @r###"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -base
    +1
    +++++++ Contents of side #2
    2
    >>>>>>> Conflict 1 of 1 ends
    "###);
    insta::assert_snapshot!(
    std::fs::read_to_string(repo_path.join("fileB")).unwrap(), @r###"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -base
    +1
    +++++++ Contents of side #2
    2
    >>>>>>> Conflict 1 of 1 ends
    "###);

    // Conflict should be simplified before being handled by external merge tool.
    check_resolve_produces_input_file(&mut test_env, &repo_path, "fileA", "base", "base\n");
    check_resolve_produces_input_file(&mut test_env, &repo_path, "fileA", "left", "1\n");
    check_resolve_produces_input_file(&mut test_env, &repo_path, "fileA", "right", "2\n");
    check_resolve_produces_input_file(&mut test_env, &repo_path, "fileB", "base", "base\n");
    check_resolve_produces_input_file(&mut test_env, &repo_path, "fileB", "left", "1\n");
    check_resolve_produces_input_file(&mut test_env, &repo_path, "fileB", "right", "2\n");

    // Check that simplified conflicts are still parsed as conflicts after editing
    // when `merge-tool-edits-conflict-markers=true`.
    let editor_script = test_env.set_up_fake_editor();
    std::fs::write(
        editor_script,
        indoc! {"
            write
            <<<<<<< Conflict 1 of 1
            %%%%%%% Changes from base to side #1
            -base_edited
            +1_edited
            +++++++ Contents of side #2
            2_edited
            >>>>>>> Conflict 1 of 1 ends
        "},
    )
    .unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &repo_path,
        &[
            "resolve",
            "--config-toml",
            "merge-tools.fake-editor.merge-tool-edits-conflict-markers=true",
            "fileB",
        ],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Resolving conflicts in: fileB
    New conflicts appeared in these commits:
      nkmrtpmo 4b14662a conflict | (conflict) conflict
    To resolve the conflicts, start by updating to it:
      jj new nkmrtpmomlro
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you may want to inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    Working copy now at: nkmrtpmo 4b14662a conflict | (conflict) conflict
    Parent commit      : kmkuslsw 18c1fb00 conflictA | (conflict) (empty) conflictA
    Parent commit      : lylxulpl d11c92eb conflictB | (conflict) (empty) conflictB
    Added 0 files, modified 1 files, removed 0 files
    There are unresolved conflicts at these paths:
    fileA    2-sided conflict
    fileB    2-sided conflict
    "###);
    insta::assert_snapshot!(std::fs::read_to_string(repo_path.join("fileB")).unwrap(), @r###"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -base_edited
    +1_edited
    +++++++ Contents of side #2
    2_edited
    >>>>>>> Conflict 1 of 1 ends
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]),
    @r###"
    fileA    2-sided conflict
    fileB    2-sided conflict
    "###);
}

#[test]
fn test_edit_delete_conflict_input_files() {
    let mut test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "base", &[], &[("file", "base\n")]);
    create_commit(&test_env, &repo_path, "a", &["base"], &[("file", "a\n")]);
    create_commit(&test_env, &repo_path, "b", &["base"], &[]);
    std::fs::remove_file(repo_path.join("file")).unwrap();
    create_commit(&test_env, &repo_path, "conflict", &["a", "b"], &[]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @    conflict
    ├─╮
    │ ○  b
    ○ │  a
    ├─╯
    ○  base
    ◆
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]), 
    @r###"
    file    2-sided conflict including 1 deletion
    "###);
    insta::assert_snapshot!(
    std::fs::read_to_string(repo_path.join("file")).unwrap()
        , @r###"
    <<<<<<< Conflict 1 of 1
    +++++++ Contents of side #1
    a
    %%%%%%% Changes from base to side #2
    -base
    >>>>>>> Conflict 1 of 1 ends
    "###);

    check_resolve_produces_input_file(&mut test_env, &repo_path, "file", "base", "base\n");
    check_resolve_produces_input_file(&mut test_env, &repo_path, "file", "left", "a\n");
    check_resolve_produces_input_file(&mut test_env, &repo_path, "file", "right", "");
}

#[test]
fn test_file_vs_dir() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "base", &[], &[("file", "base\n")]);
    create_commit(&test_env, &repo_path, "a", &["base"], &[("file", "a\n")]);
    create_commit(&test_env, &repo_path, "b", &["base"], &[]);
    std::fs::remove_file(repo_path.join("file")).unwrap();
    std::fs::create_dir(repo_path.join("file")).unwrap();
    // Without a placeholder file, `jj` ignores an empty directory
    std::fs::write(repo_path.join("file").join("placeholder"), "").unwrap();
    create_commit(&test_env, &repo_path, "conflict", &["a", "b"], &[]);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @    conflict
    ├─╮
    │ ○  b
    ○ │  a
    ├─╯
    ○  base
    ◆
    "###);

    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]), 
    @r###"
    file    2-sided conflict including a directory
    "###);
    let error = test_env.jj_cmd_failure(&repo_path, &["resolve"]);
    insta::assert_snapshot!(error, @r###"
    Hint: Using default editor ':builtin'; run `jj config set --user ui.merge-editor :builtin` to disable this message.
    Resolving conflicts in: file
    Error: Failed to resolve conflicts
    Caused by: Only conflicts that involve normal files (not symlinks, not executable, etc.) are supported. Conflict summary for "file":
    Conflict:
      Removing file with id df967b96a579e45a18b8251732d16804b2e56a55
      Adding file with id 78981922613b2afb6025042ff6bd878ac1994e85
      Adding tree with id 133bb38fc4e4bf6b551f1f04db7e48f04cac2877

    "###);
}

#[test]
fn test_description_with_dir_and_deletion() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(&test_env, &repo_path, "base", &[], &[("file", "base\n")]);
    create_commit(&test_env, &repo_path, "edit", &["base"], &[("file", "b\n")]);
    create_commit(&test_env, &repo_path, "dir", &["base"], &[]);
    std::fs::remove_file(repo_path.join("file")).unwrap();
    std::fs::create_dir(repo_path.join("file")).unwrap();
    // Without a placeholder file, `jj` ignores an empty directory
    std::fs::write(repo_path.join("file").join("placeholder"), "").unwrap();
    create_commit(&test_env, &repo_path, "del", &["base"], &[]);
    std::fs::remove_file(repo_path.join("file")).unwrap();
    create_commit(
        &test_env,
        &repo_path,
        "conflict",
        &["edit", "dir", "del"],
        &[],
    );
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @      conflict
    ├─┬─╮
    │ │ ○  del
    │ ○ │  dir
    │ ├─╯
    ○ │  edit
    ├─╯
    ○  base
    ◆
    "###);

    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]), 
    @r###"
    file    3-sided conflict including 1 deletion and a directory
    "###);
    // Test warning color. The deletion is fine, so it's not highlighted
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list", "--color=always"]), 
    @r###"
    file    [38;5;1m3-sided[38;5;3m conflict including 1 deletion and [38;5;1ma directory[39m
    "###);
    let error = test_env.jj_cmd_failure(&repo_path, &["resolve"]);
    insta::assert_snapshot!(error, @r###"
    Hint: Using default editor ':builtin'; run `jj config set --user ui.merge-editor :builtin` to disable this message.
    Resolving conflicts in: file
    Error: Failed to resolve conflicts
    Caused by: Only conflicts that involve normal files (not symlinks, not executable, etc.) are supported. Conflict summary for "file":
    Conflict:
      Removing file with id df967b96a579e45a18b8251732d16804b2e56a55
      Removing file with id df967b96a579e45a18b8251732d16804b2e56a55
      Adding file with id 61780798228d17af2d34fce4cfbdf35556832472
      Adding tree with id 133bb38fc4e4bf6b551f1f04db7e48f04cac2877

    "###);
}

#[test]
fn test_multiple_conflicts() {
    let mut test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    create_commit(
        &test_env,
        &repo_path,
        "base",
        &[],
        &[
            (
                "this_file_has_a_very_long_name_to_test_padding",
                "first base\n",
            ),
            ("another_file", "second base\n"),
        ],
    );
    create_commit(
        &test_env,
        &repo_path,
        "a",
        &["base"],
        &[
            (
                "this_file_has_a_very_long_name_to_test_padding",
                "first a\n",
            ),
            ("another_file", "second a\n"),
        ],
    );
    create_commit(
        &test_env,
        &repo_path,
        "b",
        &["base"],
        &[
            (
                "this_file_has_a_very_long_name_to_test_padding",
                "first b\n",
            ),
            ("another_file", "second b\n"),
        ],
    );
    create_commit(&test_env, &repo_path, "conflict", &["a", "b"], &[]);
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @    conflict
    ├─╮
    │ ○  b
    ○ │  a
    ├─╯
    ○  base
    ◆
    "###);
    insta::assert_snapshot!(
    std::fs::read_to_string(repo_path.join("this_file_has_a_very_long_name_to_test_padding")).unwrap()
        , @r###"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -first base
    +first a
    +++++++ Contents of side #2
    first b
    >>>>>>> Conflict 1 of 1 ends
    "###);
    insta::assert_snapshot!(
    std::fs::read_to_string(repo_path.join("another_file")).unwrap()
        , @r###"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -second base
    +second a
    +++++++ Contents of side #2
    second b
    >>>>>>> Conflict 1 of 1 ends
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]), 
    @r###"
    another_file                        2-sided conflict
    this_file_has_a_very_long_name_to_test_padding 2-sided conflict
    "###);
    // Test colors
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list", "--color=always"]), 
    @r###"
    another_file                        [38;5;3m2-sided conflict[39m
    this_file_has_a_very_long_name_to_test_padding [38;5;3m2-sided conflict[39m
    "###);

    let editor_script = test_env.set_up_fake_editor();

    // Check that we can manually pick which of the conflicts to resolve first
    std::fs::write(&editor_script, "expect\n\0write\nresolution another_file\n").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["resolve", "another_file"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Resolving conflicts in: another_file
    New conflicts appeared in these commits:
      vruxwmqv 6a90e546 conflict | (conflict) conflict
    To resolve the conflicts, start by updating to it:
      jj new vruxwmqvtpmx
    Then use `jj resolve`, or edit the conflict markers in the file directly.
    Once the conflicts are resolved, you may want to inspect the result with `jj diff`.
    Then run `jj squash` to move the resolution into the conflicted commit.
    Working copy now at: vruxwmqv 6a90e546 conflict | (conflict) conflict
    Parent commit      : zsuskuln de7553ef a | a
    Parent commit      : royxmykx f68bc2f0 b | b
    Added 0 files, modified 1 files, removed 0 files
    There are unresolved conflicts at these paths:
    this_file_has_a_very_long_name_to_test_padding 2-sided conflict
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]), 
    @r###"
    diff --git a/another_file b/another_file
    index 0000000000..a9fcc7d486 100644
    --- a/another_file
    +++ b/another_file
    @@ -1,7 +1,1 @@
    -<<<<<<< Conflict 1 of 1
    -%%%%%%% Changes from base to side #1
    --second base
    -+second a
    -+++++++ Contents of side #2
    -second b
    ->>>>>>> Conflict 1 of 1 ends
    +resolution another_file
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]), 
    @r###"
    this_file_has_a_very_long_name_to_test_padding 2-sided conflict
    "###);

    // Repeat the above with the `--quiet` option.
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    std::fs::write(&editor_script, "expect\n\0write\nresolution another_file\n").unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["resolve", "--quiet", "another_file"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @"");

    // For the rest of the test, we call `jj resolve` several times in a row to
    // resolve each conflict in the order it chooses.
    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]), 
    @"");
    std::fs::write(
        &editor_script,
        "expect\n\0write\nfirst resolution for auto-chosen file\n",
    )
    .unwrap();
    test_env.jj_cmd_ok(&repo_path, &["resolve"]);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]), 
    @r###"
    diff --git a/another_file b/another_file
    index 0000000000..7903e1c1c7 100644
    --- a/another_file
    +++ b/another_file
    @@ -1,7 +1,1 @@
    -<<<<<<< Conflict 1 of 1
    -%%%%%%% Changes from base to side #1
    --second base
    -+second a
    -+++++++ Contents of side #2
    -second b
    ->>>>>>> Conflict 1 of 1 ends
    +first resolution for auto-chosen file
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["resolve", "--list"]), 
    @r###"
    this_file_has_a_very_long_name_to_test_padding 2-sided conflict
    "###);
    std::fs::write(
        &editor_script,
        "expect\n\0write\nsecond resolution for auto-chosen file\n",
    )
    .unwrap();

    test_env.jj_cmd_ok(&repo_path, &["resolve"]);
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["diff", "--git"]), 
    @r###"
    diff --git a/another_file b/another_file
    index 0000000000..7903e1c1c7 100644
    --- a/another_file
    +++ b/another_file
    @@ -1,7 +1,1 @@
    -<<<<<<< Conflict 1 of 1
    -%%%%%%% Changes from base to side #1
    --second base
    -+second a
    -+++++++ Contents of side #2
    -second b
    ->>>>>>> Conflict 1 of 1 ends
    +first resolution for auto-chosen file
    diff --git a/this_file_has_a_very_long_name_to_test_padding b/this_file_has_a_very_long_name_to_test_padding
    index 0000000000..f8c72adf17 100644
    --- a/this_file_has_a_very_long_name_to_test_padding
    +++ b/this_file_has_a_very_long_name_to_test_padding
    @@ -1,7 +1,1 @@
    -<<<<<<< Conflict 1 of 1
    -%%%%%%% Changes from base to side #1
    --first base
    -+first a
    -+++++++ Contents of side #2
    -first b
    ->>>>>>> Conflict 1 of 1 ends
    +second resolution for auto-chosen file
    "###);

    insta::assert_snapshot!(test_env.jj_cmd_cli_error(&repo_path, &["resolve", "--list"]), 
    @r###"
    Error: No conflicts found at this revision
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_cli_error(&repo_path, &["resolve"]), 
    @r###"
    Error: No conflicts found at this revision
    "###);
}
