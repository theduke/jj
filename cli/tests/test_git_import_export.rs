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

use itertools::Itertools as _;
use jj_lib::backend::CommitId;

use crate::common::TestEnvironment;

#[test]
fn test_resolution_of_git_tracking_bookmarkes() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    test_env.jj_cmd_ok(&repo_path, &["bookmark", "create", "main"]);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-r", "main", "-m", "old_message"]);

    // Create local-git tracking bookmark
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "export"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @"");
    // Move the local bookmark somewhere else
    test_env.jj_cmd_ok(&repo_path, &["describe", "-r", "main", "-m", "new_message"]);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm b61d21b6 (empty) new_message
      @git (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 03757d22 (empty) old_message
    "###);

    // Test that we can address both revisions
    let query = |expr| {
        let template = r#"commit_id ++ " " ++ description"#;
        test_env.jj_cmd_success(
            &repo_path,
            &["log", "-r", expr, "-T", template, "--no-graph"],
        )
    };
    insta::assert_snapshot!(query("main"), @r###"
    b61d21b660c17a7191f3f73873bfe7d3f7938628 new_message
    "###);
    insta::assert_snapshot!(query("main@git"), @r###"
    03757d2212d89990ec158e97795b612a38446652 old_message
    "###);
    // Can't be selected by remote_bookmarkes()
    insta::assert_snapshot!(query(r#"remote_bookmarkes(exact:"main", exact:"git")"#), @"");
}

#[test]
fn test_git_export_conflicting_git_refs() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_ok(&repo_path, &["bookmark", "create", "main"]);
    test_env.jj_cmd_ok(&repo_path, &["bookmark", "create", "main/sub"]);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "export"]);
    insta::assert_snapshot!(stdout, @"");
    insta::with_settings!({filters => vec![("Failed to set: .*", "Failed to set: ...")]}, {
        insta::assert_snapshot!(stderr, @r###"
        Warning: Failed to export some bookmarkes:
          main/sub: Failed to set: ...
        Hint: Git doesn't allow a bookmark name that looks like a parent directory of
        another (e.g. `foo` and `foo/bar`). Try to rename the bookmarkes that failed to
        export or their "parent" bookmarkes.
        "###);
    });
}

#[test]
fn test_git_export_undo() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    let git_repo = git2::Repository::open(repo_path.join(".jj/repo/store/git")).unwrap();

    test_env.jj_cmd_ok(&repo_path, &["bookmark", "create", "a"]);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    a: qpvuntsm 230dd059 (empty) (no description set)
    "###);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "export"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @"");
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["log", "-ra@git"]), @r###"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 a 230dd059
    │  (empty) (no description set)
    ~
    "###);

    // Exported refs won't be removed by undoing the export, but the git-tracking
    // bookmark is. This is the same as remote-tracking bookmarkes.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "undo"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @"");
    insta::assert_debug_snapshot!(get_git_repo_refs(&git_repo), @r###"
    [
        (
            "refs/heads/a",
            CommitId(
                "230dd059e1b059aefc0da06a2e5a7dbf22362f22",
            ),
        ),
    ]
    "###);
    insta::assert_snapshot!(test_env.jj_cmd_failure(&repo_path, &["log", "-ra@git"]), @r###"
    Error: Revision "a@git" doesn't exist
    Hint: Did you mean "a"?
    "###);

    // This would re-export bookmark "a" and create git-tracking bookmark.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "export"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @"");
    insta::assert_snapshot!(test_env.jj_cmd_success(&repo_path, &["log", "-ra@git"]), @r###"
    @  qpvuntsm test.user@example.com 2001-02-03 08:05:07 a 230dd059
    │  (empty) (no description set)
    ~
    "###);
}

#[test]
fn test_git_import_undo() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    let git_repo = git2::Repository::open(repo_path.join(".jj/repo/store/git")).unwrap();

    // Create bookmark "a" in git repo
    let commit_id =
        test_env.jj_cmd_success(&repo_path, &["log", "-Tcommit_id", "--no-graph", "-r@"]);
    let commit = git_repo
        .find_commit(git2::Oid::from_str(&commit_id).unwrap())
        .unwrap();
    git_repo.bookmark("a", &commit, true).unwrap();

    // Initial state we will return to after `undo`. There are no bookmarkes.
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    let base_operation_id = test_env.current_operation_id(&repo_path);

    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "import"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    bookmark: a [new] tracked
    "###);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    a: qpvuntsm 230dd059 (empty) (no description set)
      @git: qpvuntsm 230dd059 (empty) (no description set)
    "###);

    // "git import" can be undone by default.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "restore", &base_operation_id]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @"");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    // Try "git import" again, which should re-import the bookmark "a".
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "import"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    bookmark: a [new] tracked
    "###);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    a: qpvuntsm 230dd059 (empty) (no description set)
      @git: qpvuntsm 230dd059 (empty) (no description set)
    "###);
}

#[test]
fn test_git_import_move_export_with_default_undo() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");
    let git_repo = git2::Repository::open(repo_path.join(".jj/repo/store/git")).unwrap();

    // Create bookmark "a" in git repo
    let commit_id =
        test_env.jj_cmd_success(&repo_path, &["log", "-Tcommit_id", "--no-graph", "-r@"]);
    let commit = git_repo
        .find_commit(git2::Oid::from_str(&commit_id).unwrap())
        .unwrap();
    git_repo.bookmark("a", &commit, true).unwrap();

    // Initial state we will try to return to after `op restore`. There are no
    // bookmarkes.
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    let base_operation_id = test_env.current_operation_id(&repo_path);

    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "import"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    bookmark: a [new] tracked
    "###);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    a: qpvuntsm 230dd059 (empty) (no description set)
      @git: qpvuntsm 230dd059 (empty) (no description set)
    "###);

    // Move bookmark "a" and export to git repo
    test_env.jj_cmd_ok(&repo_path, &["new"]);
    test_env.jj_cmd_ok(&repo_path, &["bookmark", "set", "a"]);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    a: yqosqzyt 096dc80d (empty) (no description set)
      @git (behind by 1 commits): qpvuntsm 230dd059 (empty) (no description set)
    "###);
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "export"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @"");
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    a: yqosqzyt 096dc80d (empty) (no description set)
      @git: yqosqzyt 096dc80d (empty) (no description set)
    "###);

    // "git import" can be undone with the default `restore` behavior, as shown in
    // the previous test. However, "git export" can't: the bookmarkes in the git
    // repo stay where they were.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["op", "restore", &base_operation_id]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Working copy now at: qpvuntsm 230dd059 (empty) (no description set)
    Parent commit      : zzzzzzzz 00000000 (empty) (no description set)
    "###);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @"");
    insta::assert_debug_snapshot!(get_git_repo_refs(&git_repo), @r###"
    [
        (
            "refs/heads/a",
            CommitId(
                "096dc80da67094fbaa6683e2a205dddffa31f9a8",
            ),
        ),
    ]
    "###);

    // The last bookmark "a" state is imported from git. No idea what's the most
    // intuitive result here.
    let (stdout, stderr) = test_env.jj_cmd_ok(&repo_path, &["git", "import"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    bookmark: a [new] tracked
    "###);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    a: yqosqzyt 096dc80d (empty) (no description set)
      @git: yqosqzyt 096dc80d (empty) (no description set)
    "###);
}

fn get_bookmark_output(test_env: &TestEnvironment, repo_path: &Path) -> String {
    test_env.jj_cmd_success(repo_path, &["bookmark", "list", "--all-remotes"])
}

fn get_git_repo_refs(git_repo: &git2::Repository) -> Vec<(String, CommitId)> {
    let mut refs: Vec<_> = git_repo
        .references()
        .unwrap()
        .filter_ok(|git_ref| git_ref.is_tag() || git_ref.is_bookmark() || git_ref.is_remote())
        .filter_map_ok(|git_ref| {
            let full_name = git_ref.name()?.to_owned();
            let git_commit = git_ref.peel_to_commit().ok()?;
            let commit_id = CommitId::from_bytes(git_commit.id().as_bytes());
            Some((full_name, commit_id))
        })
        .try_collect()
        .unwrap();
    refs.sort();
    refs
}
