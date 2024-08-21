// Copyright 2024 The Jujutsu Authors
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

use test_case::test_case;

use crate::common::TestEnvironment;

fn get_log_output_with_bookmarkes(test_env: &TestEnvironment, cwd: &Path) -> String {
    // Don't include commit IDs since they will be different depending on
    // whether the test runs with `jj commit` or `jj describe` + `jj new`.
    let template = r#""bookmarkes{" ++ local_bookmarkes ++ "} desc: " ++ description"#;
    test_env.jj_cmd_success(cwd, &["log", "-T", template])
}

fn set_advance_bookmarkes(test_env: &TestEnvironment, enabled: bool) {
    if enabled {
        test_env.add_config(
            r#"[experimental-advance-bookmarkes]
        enabled-bookmarkes = ["glob:*"]
        "#,
        );
    } else {
        test_env.add_config(
            r#"[experimental-advance-bookmarkes]
        enabled-bookmarkes = []
        "#,
        );
    }
}

// Runs a command in the specified test environment and workspace path that
// describes the current commit with `commit_message` and creates a new commit
// on top of it.
type CommitFn = fn(env: &TestEnvironment, workspace_path: &Path, commit_message: &str);

// Implements CommitFn using the `jj commit` command.
fn commit_cmd(env: &TestEnvironment, workspace_path: &Path, commit_message: &str) {
    env.jj_cmd_ok(workspace_path, &["commit", "-m", commit_message]);
}

// Implements CommitFn using the `jj describe` and `jj new`.
fn describe_new_cmd(env: &TestEnvironment, workspace_path: &Path, commit_message: &str) {
    env.jj_cmd_ok(workspace_path, &["describe", "-m", commit_message]);
    env.jj_cmd_ok(workspace_path, &["new"]);
}

// Check that enabling and disabling advance-bookmarkes works as expected.
#[test_case(commit_cmd ; "commit")]
#[test_case(describe_new_cmd; "new")]
fn test_advance_bookmarkes_enabled(make_commit: CommitFn) {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let workspace_path = test_env.env_root().join("repo");

    // First, test with advance-bookmarkes enabled. Start by creating a bookmark on
    // the root commit.
    set_advance_bookmarkes(&test_env, true);
    test_env.jj_cmd_ok(
        &workspace_path,
        &["bookmark", "create", "-r", "@-", "test_bookmark"],
    );

    // Check the initial state of the repo.
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ◆  bookmarkes{test_bookmark} desc:
    "###);
    }

    // Run jj commit, which will advance the bookmark pointing to @-.
    make_commit(&test_env, &workspace_path, "first");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{test_bookmark} desc: first
    ◆  bookmarkes{} desc:
    "###);
    }

    // Now disable advance bookmarkes and commit again. The bookmark shouldn't move.
    set_advance_bookmarkes(&test_env, false);
    make_commit(&test_env, &workspace_path, "second");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{} desc: second
    ○  bookmarkes{test_bookmark} desc: first
    ◆  bookmarkes{} desc:
    "###);
    }
}

// Check that only a bookmark pointing to @- advances. Branches pointing to @
// are not advanced.
#[test_case(commit_cmd ; "commit")]
#[test_case(describe_new_cmd; "new")]
fn test_advance_bookmarkes_at_minus(make_commit: CommitFn) {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let workspace_path = test_env.env_root().join("repo");

    set_advance_bookmarkes(&test_env, true);
    test_env.jj_cmd_ok(&workspace_path, &["bookmark", "create", "test_bookmark"]);

    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{test_bookmark} desc:
    ◆  bookmarkes{} desc:
    "###);
    }

    make_commit(&test_env, &workspace_path, "first");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{test_bookmark} desc: first
    ◆  bookmarkes{} desc:
    "###);
    }

    // Create a second bookmark pointing to @. On the next commit, only the first
    // bookmark, which points to @-, will advance.
    test_env.jj_cmd_ok(&workspace_path, &["bookmark", "create", "test_bookmark2"]);
    make_commit(&test_env, &workspace_path, "second");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{test_bookmark test_bookmark2} desc: second
    ○  bookmarkes{} desc: first
    ◆  bookmarkes{} desc:
    "###);
    }
}

// Test that per-bookmark overrides invert the behavior of
// experimental-advance-bookmarkes.enabled.
#[test_case(commit_cmd ; "commit")]
#[test_case(describe_new_cmd; "new")]
fn test_advance_bookmarkes_overrides(make_commit: CommitFn) {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let workspace_path = test_env.env_root().join("repo");

    // advance-bookmarkes is disabled by default.
    test_env.jj_cmd_ok(
        &workspace_path,
        &["bookmark", "create", "-r", "@-", "test_bookmark"],
    );

    // Check the initial state of the repo.
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ◆  bookmarkes{test_bookmark} desc:
    "###);
    }

    // Commit will not advance the bookmark since advance-bookmarkes is disabled.
    make_commit(&test_env, &workspace_path, "first");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{} desc: first
    ◆  bookmarkes{test_bookmark} desc:
    "###);
    }

    // Now enable advance bookmarkes for "test_bookmark", move the bookmark, and
    // commit again.
    test_env.add_config(
        r#"[experimental-advance-bookmarkes]
    enabled-bookmarkes = ["test_bookmark"]
    "#,
    );
    test_env.jj_cmd_ok(
        &workspace_path,
        &["bookmark", "set", "test_bookmark", "-r", "@-"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{test_bookmark} desc: first
    ◆  bookmarkes{} desc:
    "###);
    }
    make_commit(&test_env, &workspace_path, "second");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{test_bookmark} desc: second
    ○  bookmarkes{} desc: first
    ◆  bookmarkes{} desc:
    "###);
    }

    // Now disable advance bookmarkes for "test_bookmark" and "second_bookmark",
    // which we will use later. Disabling always takes precedence over enabling.
    test_env.add_config(
        r#"[experimental-advance-bookmarkes]
    enabled-bookmarkes = ["test_bookmark", "second_bookmark"]
    disabled-bookmarkes = ["test_bookmark"]
    "#,
    );
    make_commit(&test_env, &workspace_path, "third");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{} desc: third
    ○  bookmarkes{test_bookmark} desc: second
    ○  bookmarkes{} desc: first
    ◆  bookmarkes{} desc:
    "###);
    }

    // If we create a new bookmark at @- and move test_bookmark there as well. When
    // we commit, only "second_bookmark" will advance since "test_bookmark" is
    // disabled.
    test_env.jj_cmd_ok(
        &workspace_path,
        &["bookmark", "create", "second_bookmark", "-r", "@-"],
    );
    test_env.jj_cmd_ok(
        &workspace_path,
        &["bookmark", "set", "test_bookmark", "-r", "@-"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{second_bookmark test_bookmark} desc: third
    ○  bookmarkes{} desc: second
    ○  bookmarkes{} desc: first
    ◆  bookmarkes{} desc:
    "###);
    }
    make_commit(&test_env, &workspace_path, "fourth");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{second_bookmark} desc: fourth
    ○  bookmarkes{test_bookmark} desc: third
    ○  bookmarkes{} desc: second
    ○  bookmarkes{} desc: first
    ◆  bookmarkes{} desc:
    "###);
    }
}

// If multiple eligible bookmarkes point to @-, all of them will be advanced.
#[test_case(commit_cmd ; "commit")]
#[test_case(describe_new_cmd; "new")]
fn test_advance_bookmarkes_multiple_bookmarkes(make_commit: CommitFn) {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let workspace_path = test_env.env_root().join("repo");

    set_advance_bookmarkes(&test_env, true);
    test_env.jj_cmd_ok(
        &workspace_path,
        &["bookmark", "create", "-r", "@-", "first_bookmark"],
    );
    test_env.jj_cmd_ok(
        &workspace_path,
        &["bookmark", "create", "-r", "@-", "second_bookmark"],
    );

    insta::allow_duplicates! {
    // Check the initial state of the repo.
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ◆  bookmarkes{first_bookmark second_bookmark} desc:
    "###);
    }

    // Both bookmarkes are eligible and both will advance.
    make_commit(&test_env, &workspace_path, "first");
    insta::allow_duplicates! {
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{first_bookmark second_bookmark} desc: first
    ◆  bookmarkes{} desc:
    "###);
    }
}

// Call `jj new` on an interior commit and see that the bookmark pointing to its
// parent's parent is advanced.
#[test]
fn test_new_advance_bookmarkes_interior() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let workspace_path = test_env.env_root().join("repo");

    set_advance_bookmarkes(&test_env, true);

    // Check the initial state of the repo.
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ◆  bookmarkes{} desc:
    "###);

    // Create a gap in the commits for us to insert our new commit with --before.
    test_env.jj_cmd_ok(&workspace_path, &["commit", "-m", "first"]);
    test_env.jj_cmd_ok(&workspace_path, &["commit", "-m", "second"]);
    test_env.jj_cmd_ok(&workspace_path, &["commit", "-m", "third"]);
    test_env.jj_cmd_ok(
        &workspace_path,
        &["bookmark", "create", "-r", "@---", "test_bookmark"],
    );
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{} desc: third
    ○  bookmarkes{} desc: second
    ○  bookmarkes{test_bookmark} desc: first
    ◆  bookmarkes{} desc:
    "###);

    test_env.jj_cmd_ok(&workspace_path, &["new", "-r", "@--"]);
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    │ ○  bookmarkes{} desc: third
    ├─╯
    ○  bookmarkes{test_bookmark} desc: second
    ○  bookmarkes{} desc: first
    ◆  bookmarkes{} desc:
    "###);
}

// If the `--before` flag is passed to `jj new`, bookmarkes are not advanced.
#[test]
fn test_new_advance_bookmarkes_before() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let workspace_path = test_env.env_root().join("repo");

    set_advance_bookmarkes(&test_env, true);

    // Check the initial state of the repo.
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ◆  bookmarkes{} desc:
    "###);

    // Create a gap in the commits for us to insert our new commit with --before.
    test_env.jj_cmd_ok(&workspace_path, &["commit", "-m", "first"]);
    test_env.jj_cmd_ok(&workspace_path, &["commit", "-m", "second"]);
    test_env.jj_cmd_ok(&workspace_path, &["commit", "-m", "third"]);
    test_env.jj_cmd_ok(
        &workspace_path,
        &["bookmark", "create", "-r", "@---", "test_bookmark"],
    );
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{} desc: third
    ○  bookmarkes{} desc: second
    ○  bookmarkes{test_bookmark} desc: first
    ◆  bookmarkes{} desc:
    "###);

    test_env.jj_cmd_ok(&workspace_path, &["new", "--before", "@-"]);
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    ○  bookmarkes{} desc: third
    @  bookmarkes{} desc:
    ○  bookmarkes{} desc: second
    ○  bookmarkes{test_bookmark} desc: first
    ◆  bookmarkes{} desc:
    "###);
}

// If the `--after` flag is passed to `jj new`, bookmarkes are not advanced.
#[test]
fn test_new_advance_bookmarkes_after() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let workspace_path = test_env.env_root().join("repo");

    set_advance_bookmarkes(&test_env, true);
    test_env.jj_cmd_ok(
        &workspace_path,
        &["bookmark", "create", "-r", "@-", "test_bookmark"],
    );

    // Check the initial state of the repo.
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ◆  bookmarkes{test_bookmark} desc:
    "###);

    test_env.jj_cmd_ok(&workspace_path, &["describe", "-m", "first"]);
    test_env.jj_cmd_ok(&workspace_path, &["new", "--after", "@"]);
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc:
    ○  bookmarkes{} desc: first
    ◆  bookmarkes{test_bookmark} desc:
    "###);
}

#[test]
fn test_new_advance_bookmarkes_merge_children() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let workspace_path = test_env.env_root().join("repo");

    set_advance_bookmarkes(&test_env, true);
    test_env.jj_cmd_ok(&workspace_path, &["desc", "-m", "0"]);
    test_env.jj_cmd_ok(&workspace_path, &["new", "-m", "1"]);
    test_env.jj_cmd_ok(&workspace_path, &["new", "description(0)", "-m", "2"]);
    test_env.jj_cmd_ok(
        &workspace_path,
        &[
            "bookmark",
            "create",
            "test_bookmark",
            "-r",
            "description(0)",
        ],
    );

    // Check the initial state of the repo.
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @  bookmarkes{} desc: 2
    │ ○  bookmarkes{} desc: 1
    ├─╯
    ○  bookmarkes{test_bookmark} desc: 0
    ◆  bookmarkes{} desc:
    "###);

    // The bookmark won't advance because `jj  new` had multiple targets.
    test_env.jj_cmd_ok(
        &workspace_path,
        &["new", "description(1)", "description(2)"],
    );
    insta::assert_snapshot!(get_log_output_with_bookmarkes(&test_env, &workspace_path), @r###"
    @    bookmarkes{} desc:
    ├─╮
    │ ○  bookmarkes{} desc: 2
    ○ │  bookmarkes{} desc: 1
    ├─╯
    ○  bookmarkes{test_bookmark} desc: 0
    ◆  bookmarkes{} desc:
    "###);
}
