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

use itertools::Itertools as _;
use jj_lib::git;

use super::find_remote_bookmarkes;
use crate::cli_util::CommandHelper;
use crate::cli_util::RemoteBookmarkNamePattern;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Stop tracking given remote bookmarkes
///
/// A non-tracking remote bookmark is just a pointer to the last-fetched remote
/// bookmark. It won't be imported as a local bookmark on future pulls.
#[derive(clap::Args, Clone, Debug)]
pub struct BookmarkUntrackArgs {
    /// Remote bookmarkes to untrack
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select bookmarkes by wildcard pattern. For details, see
    /// https://github.com/martinvonz/jj/blob/main/docs/revsets.md#string-patterns.
    ///
    /// Examples: bookmark@remote, glob:main@*, glob:jjfan-*@upstream
    #[arg(required = true, value_name = "BRANCH@REMOTE")]
    names: Vec<RemoteBookmarkNamePattern>,
}

pub fn cmd_bookmark_untrack(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkUntrackArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let view = workspace_command.repo().view();
    let mut names = Vec::new();
    for (name, remote_ref) in find_remote_bookmarkes(view, &args.names)? {
        if name.remote == git::REMOTE_NAME_FOR_LOCAL_GIT_REPO {
            // This restriction can be lifted if we want to support untracked @git
            // bookmarkes.
            writeln!(
                ui.warning_default(),
                "Git-tracking bookmark cannot be untracked: {name}"
            )?;
        } else if !remote_ref.is_tracking() {
            writeln!(
                ui.warning_default(),
                "Remote bookmark not tracked yet: {name}"
            )?;
        } else {
            names.push(name);
        }
    }
    let mut tx = workspace_command.start_transaction();
    for name in &names {
        tx.mut_repo()
            .untrack_remote_bookmark(&name.bookmark, &name.remote);
    }
    if !names.is_empty() {
        writeln!(
            ui.status(),
            "Stopped tracking {} remote bookmarkes.",
            names.len()
        )?;
    }
    tx.finish(
        ui,
        format!("untrack remote bookmark {}", names.iter().join(", ")),
    )?;
    Ok(())
}
