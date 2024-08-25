// Copyright 2023 The Jujutsu Authors
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

use std::path::PathBuf;

use crate::common::TestEnvironment;

fn set_up(trunk_name: &str) -> (TestEnvironment, PathBuf) {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "origin"]);
    let origin_path = test_env.env_root().join("origin");
    let origin_git_repo_path = origin_path
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");

    test_env.jj_cmd_ok(&origin_path, &["describe", "-m=description 1"]);
    test_env.jj_cmd_ok(&origin_path, &["bookmark", "create", trunk_name]);
    test_env.jj_cmd_ok(&origin_path, &["new", "root()", "-m=description 2"]);
    test_env.jj_cmd_ok(&origin_path, &["bookmark", "create", "unrelated_bookmark"]);
    test_env.jj_cmd_ok(&origin_path, &["git", "export"]);

    test_env.jj_cmd_ok(
        test_env.env_root(),
        &[
            "git",
            "clone",
            "--config-toml=git.auto-local-bookmark=true",
            origin_git_repo_path.to_str().unwrap(),
            "local",
        ],
    );
    let workspace_root = test_env.env_root().join("local");
    (test_env, workspace_root)
}

#[test]
fn test_builtin_alias_trunk_matches_main() {
    let (test_env, workspace_root) = set_up("main");

    let stdout = test_env.jj_cmd_success(&workspace_root, &["log", "-r", "trunk()"]);
    insta::assert_snapshot!(stdout, @r###"
    ◆  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 main d13ecdbd
    │  (empty) description 1
    ~
    "###);
}

#[test]
fn test_builtin_alias_trunk_matches_master() {
    let (test_env, workspace_root) = set_up("master");

    let stdout = test_env.jj_cmd_success(&workspace_root, &["log", "-r", "trunk()"]);
    insta::assert_snapshot!(stdout, @r###"
    ◆  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 master d13ecdbd
    │  (empty) description 1
    ~
    "###);
}

#[test]
fn test_builtin_alias_trunk_matches_trunk() {
    let (test_env, workspace_root) = set_up("trunk");

    let stdout = test_env.jj_cmd_success(&workspace_root, &["log", "-r", "trunk()"]);
    insta::assert_snapshot!(stdout, @r###"
    ◆  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 trunk d13ecdbd
    │  (empty) description 1
    ~
    "###);
}

#[test]
fn test_builtin_alias_trunk_matches_exactly_one_commit() {
    let (test_env, workspace_root) = set_up("main");
    let origin_path = test_env.env_root().join("origin");
    test_env.jj_cmd_ok(&origin_path, &["new", "root()", "-m=description 3"]);
    test_env.jj_cmd_ok(&origin_path, &["bookmark", "create", "master"]);

    let stdout = test_env.jj_cmd_success(&workspace_root, &["log", "-r", "trunk()"]);
    insta::assert_snapshot!(stdout, @r###"
    ◆  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 main d13ecdbd
    │  (empty) description 1
    ~
    "###);
}

#[test]
fn test_builtin_alias_trunk_override_alias() {
    let (test_env, workspace_root) = set_up("override-trunk");

    test_env.add_config(
        r#"revset-aliases.'trunk()' = 'latest(remote_bookmarkes(exact:"override-trunk", exact:"origin"))'"#,
    );

    let stdout = test_env.jj_cmd_success(&workspace_root, &["log", "-r", "trunk()"]);
    insta::assert_snapshot!(stdout, @r###"
    ◆  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 override-trunk d13ecdbd
    │  (empty) description 1
    ~
    "###);
}

#[test]
fn test_builtin_alias_trunk_no_match() {
    let (test_env, workspace_root) = set_up("no-match-trunk");

    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["log", "-r", "trunk()"]);
    insta::assert_snapshot!(stdout, @r###"
    ◆  zzzzzzzz root() 00000000
    "###);
    insta::assert_snapshot!(stderr, @r###"
    "###);
}

#[test]
fn test_builtin_alias_trunk_no_match_only_exact() {
    let (test_env, workspace_root) = set_up("maint");

    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["log", "-r", "trunk()"]);
    insta::assert_snapshot!(stdout, @r###"
    ◆  zzzzzzzz root() 00000000
    "###);
    insta::assert_snapshot!(stderr, @r###"
    "###);
}

#[test]
fn test_builtin_user_redefines_builtin_immutable_heads() {
    let (test_env, workspace_root) = set_up("main");

    test_env.add_config(r#"revset-aliases.'builtin_immutable_heads()' = '@'"#);
    test_env.add_config(r#"revset-aliases.'mutable()' = '@'"#);
    test_env.add_config(r#"revset-aliases.'immutable()' = '@'"#);

    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["log", "-r", "trunk()"]);
    insta::assert_snapshot!(stdout, @r###"
    ○  xtvrqkyv test.user@example.com 2001-02-03 08:05:08 main d13ecdbd
    │  (empty) description 1
    ~
    "###);
    insta::assert_snapshot!(stderr, @r###"
    Warning: Redefining `revset-aliases.builtin_immutable_heads()` is not recommended; redefine `immutable_heads()` instead
    Warning: Redefining `revset-aliases.immutable()` is not recommended; redefine `immutable_heads()` instead
    Warning: Redefining `revset-aliases.mutable()` is not recommended; redefine `immutable_heads()` instead
    "###);
}
