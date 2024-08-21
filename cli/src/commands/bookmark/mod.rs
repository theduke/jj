// Copyright 2020-2023 The Jujutsu Authors
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

mod create;
mod delete;
mod forget;
mod list;
mod r#move;
mod rename;
mod set;
mod track;
mod untrack;

use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::git;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::repo::Repo;
use jj_lib::str_util::StringPattern;
use jj_lib::view::View;

use self::create::cmd_bookmark_create;
use self::create::BranchCreateArgs;
use self::delete::cmd_bookmark_delete;
use self::delete::BranchDeleteArgs;
use self::forget::cmd_bookmark_forget;
use self::forget::BranchForgetArgs;
use self::list::cmd_bookmark_list;
use self::list::BranchListArgs;
use self::r#move::cmd_bookmark_move;
use self::r#move::BranchMoveArgs;
use self::rename::cmd_bookmark_rename;
use self::rename::BranchRenameArgs;
use self::set::cmd_bookmark_set;
use self::set::BranchSetArgs;
use self::track::cmd_bookmark_track;
use self::track::BranchTrackArgs;
use self::untrack::cmd_bookmark_untrack;
use self::untrack::BranchUntrackArgs;
use crate::cli_util::CommandHelper;
use crate::cli_util::RemoteBranchName;
use crate::cli_util::RemoteBranchNamePattern;
use crate::command_error::user_error;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Manage bookmarks
///
/// For information about bookmarks, see
/// https://github.com/martinvonz/jj/blob/main/docs/bookmarks.md.
#[derive(clap::Subcommand, Clone, Debug)]
pub enum BranchCommand {
    #[command(visible_alias("c"))]
    Create(BranchCreateArgs),
    #[command(visible_alias("d"))]
    Delete(BranchDeleteArgs),
    #[command(visible_alias("f"))]
    Forget(BranchForgetArgs),
    #[command(visible_alias("l"))]
    List(BranchListArgs),
    #[command(visible_alias("m"))]
    Move(BranchMoveArgs),
    #[command(visible_alias("r"))]
    Rename(BranchRenameArgs),
    #[command(visible_alias("s"))]
    Set(BranchSetArgs),
    #[command(visible_alias("t"))]
    Track(BranchTrackArgs),
    Untrack(BranchUntrackArgs),
}

pub fn cmd_bookmark(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &BranchCommand,
) -> Result<(), CommandError> {
    match subcommand {
        BranchCommand::Create(args) => cmd_bookmark_create(ui, command, args),
        BranchCommand::Delete(args) => cmd_bookmark_delete(ui, command, args),
        BranchCommand::Forget(args) => cmd_bookmark_forget(ui, command, args),
        BranchCommand::List(args) => cmd_bookmark_list(ui, command, args),
        BranchCommand::Move(args) => cmd_bookmark_move(ui, command, args),
        BranchCommand::Rename(args) => cmd_bookmark_rename(ui, command, args),
        BranchCommand::Set(args) => cmd_bookmark_set(ui, command, args),
        BranchCommand::Track(args) => cmd_bookmark_track(ui, command, args),
        BranchCommand::Untrack(args) => cmd_bookmark_untrack(ui, command, args),
    }
}

fn find_local_bookmarks<'a>(
    view: &'a View,
    name_patterns: &[StringPattern],
) -> Result<Vec<(&'a str, &'a RefTarget)>, CommandError> {
    find_bookmarks_with(name_patterns, |pattern| {
        view.local_bookmarks_matching(pattern)
    })
}

fn find_bookmarks_with<'a, 'b, V, I: Iterator<Item = (&'a str, V)>>(
    name_patterns: &'b [StringPattern],
    mut find_matches: impl FnMut(&'b StringPattern) -> I,
) -> Result<Vec<I::Item>, CommandError> {
    let mut matching_bookmarks: Vec<I::Item> = vec![];
    let mut unmatched_patterns = vec![];
    for pattern in name_patterns {
        let mut matches = find_matches(pattern).peekable();
        if matches.peek().is_none() {
            unmatched_patterns.push(pattern);
        }
        matching_bookmarks.extend(matches);
    }
    match &unmatched_patterns[..] {
        [] => {
            matching_bookmarks.sort_unstable_by_key(|(name, _)| *name);
            matching_bookmarks.dedup_by_key(|(name, _)| *name);
            Ok(matching_bookmarks)
        }
        [pattern] if pattern.is_exact() => Err(user_error(format!("No such bookmark: {pattern}"))),
        patterns => Err(user_error(format!(
            "No matching bookmarks for patterns: {}",
            patterns.iter().join(", ")
        ))),
    }
}

fn find_remote_bookmarks<'a>(
    view: &'a View,
    name_patterns: &[RemoteBranchNamePattern],
) -> Result<Vec<(RemoteBranchName, &'a RemoteRef)>, CommandError> {
    let mut matching_bookmarks = vec![];
    let mut unmatched_patterns = vec![];
    for pattern in name_patterns {
        let mut matches = view
            .remote_bookmarks_matching(&pattern.bookmark, &pattern.remote)
            .map(|((bookmark, remote), remote_ref)| {
                let name = RemoteBranchName {
                    bookmark: bookmark.to_owned(),
                    remote: remote.to_owned(),
                };
                (name, remote_ref)
            })
            .peekable();
        if matches.peek().is_none() {
            unmatched_patterns.push(pattern);
        }
        matching_bookmarks.extend(matches);
    }
    match &unmatched_patterns[..] {
        [] => {
            matching_bookmarks.sort_unstable_by(|(name1, _), (name2, _)| name1.cmp(name2));
            matching_bookmarks.dedup_by(|(name1, _), (name2, _)| name1 == name2);
            Ok(matching_bookmarks)
        }
        [pattern] if pattern.is_exact() => {
            Err(user_error(format!("No such remote bookmark: {pattern}")))
        }
        patterns => Err(user_error(format!(
            "No matching remote bookmarks for patterns: {}",
            patterns.iter().join(", ")
        ))),
    }
}

/// Whether or not the `bookmark` has any tracked remotes (i.e. is a tracking
/// local bookmark.)
fn has_tracked_remote_bookmarks(view: &View, bookmark: &str) -> bool {
    view.remote_bookmarks_matching(
        &StringPattern::exact(bookmark),
        &StringPattern::everything(),
    )
    .filter(|&((_, remote_name), _)| remote_name != git::REMOTE_NAME_FOR_LOCAL_GIT_REPO)
    .any(|(_, remote_ref)| remote_ref.is_tracking())
}

fn is_fast_forward(repo: &dyn Repo, old_target: &RefTarget, new_target_id: &CommitId) -> bool {
    if old_target.is_present() {
        // Strictly speaking, "all" old targets should be ancestors, but we allow
        // conflict resolution by setting bookmark to "any" of the old target
        // descendants.
        old_target
            .added_ids()
            .any(|old| repo.index().is_ancestor(old, new_target_id))
    } else {
        true
    }
}
