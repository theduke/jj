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

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::io;
use std::io::Write;

use clap::ArgGroup;
use itertools::Itertools;
use jj_lib::backend::CommitId;
use jj_lib::git;
use jj_lib::git::GitBranchPushTargets;
use jj_lib::git::GitPushError;
use jj_lib::object_id::ObjectId;
use jj_lib::op_store::RefTarget;
use jj_lib::refs::classify_bookmark_push_action;
use jj_lib::refs::BranchPushAction;
use jj_lib::refs::BranchPushUpdate;
use jj_lib::refs::LocalAndRemoteRef;
use jj_lib::repo::Repo;
use jj_lib::revset::RevsetExpression;
use jj_lib::settings::ConfigResultExt as _;
use jj_lib::settings::UserSettings;
use jj_lib::str_util::StringPattern;
use jj_lib::view::View;

use crate::cli_util::short_change_hash;
use crate::cli_util::short_commit_hash;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::command_error::user_error;
use crate::command_error::user_error_with_hint;
use crate::command_error::CommandError;
use crate::commands::git::get_single_remote;
use crate::commands::git::map_git_error;
use crate::git_util::get_git_repo;
use crate::git_util::with_remote_git_callbacks;
use crate::git_util::GitSidebandProgressMessageWriter;
use crate::revset_util;
use crate::ui::Ui;

/// Push to a Git remote
///
/// By default, pushes any bookmarkes pointing to
/// `remote_bookmarkes(remote=<remote>)..@`. Use `--bookmark` to push specific
/// bookmarkes. Use `--all` to push all bookmarkes. Use `--change` to generate
/// bookmark names based on the change IDs of specific commits.
///
/// Before the command actually moves, creates, or deletes a remote bookmark, it
/// makes several [safety checks]. If there is a problem, you may need to run
/// `jj git fetch --remote <remote name>` and/or resolve some [bookmark
/// conflicts].
///
/// [safety checks]:
///     https://martinvonz.github.io/jj/latest/bookmarkes/#pushing-bookmarkes-safety-checks
///
/// [bookmark conflicts]:
///     https://martinvonz.github.io/jj/latest/bookmarkes/#conflicts

#[derive(clap::Args, Clone, Debug)]
#[command(group(ArgGroup::new("specific").args(&["bookmark", "change", "revisions"]).multiple(true)))]
#[command(group(ArgGroup::new("what").args(&["all", "deleted", "tracked"]).conflicts_with("specific")))]
pub struct GitPushArgs {
    /// The remote to push to (only named remotes are supported)
    #[arg(long)]
    remote: Option<String>,
    /// Push only this bookmark, or bookmarkes matching a pattern (can be
    /// repeated)
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select bookmarkes by wildcard pattern. For details, see
    /// https://martinvonz.github.io/jj/latest/revsets#string-patterns.
    #[arg(long, short, value_parser = StringPattern::parse)]
    bookmark: Vec<StringPattern>,
    /// Push all bookmarkes (including deleted bookmarkes)
    #[arg(long)]
    all: bool,
    /// Push all tracked bookmarkes (including deleted bookmarkes)
    ///
    /// This usually means that the bookmark was already pushed to or fetched
    /// from the relevant remote. For details, see
    /// https://martinvonz.github.io/jj/latest/bookmarkes#remotes-and-tracked-bookmarkes
    #[arg(long)]
    tracked: bool,
    /// Push all deleted bookmarkes
    ///
    /// Only tracked bookmarkes can be successfully deleted on the remote. A
    /// warning will be printed if any untracked bookmarkes on the remote
    /// correspond to missing local bookmarkes.
    #[arg(long)]
    deleted: bool,
    /// Allow pushing commits with empty descriptions
    #[arg(long)]
    allow_empty_description: bool,
    /// Allow pushing commits that are private
    #[arg(long)]
    allow_private: bool,
    /// Push bookmarkes pointing to these commits (can be repeated)
    #[arg(long, short)]
    revisions: Vec<RevisionArg>,
    /// Push this commit by creating a bookmark based on its change ID (can be
    /// repeated)
    #[arg(long, short)]
    change: Vec<RevisionArg>,
    /// Only display what will change on the remote
    #[arg(long)]
    dry_run: bool,
}

fn make_bookmark_term(bookmark_names: &[impl fmt::Display]) -> String {
    match bookmark_names {
        [bookmark_name] => format!("bookmark {}", bookmark_name),
        bookmark_names => format!("bookmarkes {}", bookmark_names.iter().join(", ")),
    }
}

const DEFAULT_REMOTE: &str = "origin";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BranchMoveDirection {
    Forward,
    Backward,
    Sideways,
}

pub fn cmd_git_push(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitPushArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let git_repo = get_git_repo(workspace_command.repo().store())?;

    let remote = if let Some(name) = &args.remote {
        name.clone()
    } else {
        get_default_push_remote(ui, command.settings(), &git_repo)?
    };

    let repo = workspace_command.repo().clone();
    let mut tx = workspace_command.start_transaction();
    let tx_description;
    let mut bookmark_updates = vec![];
    if args.all {
        for (bookmark_name, targets) in repo.view().local_remote_bookmarkes(&remote) {
            match classify_bookmark_update(bookmark_name, &remote, targets) {
                Ok(Some(update)) => bookmark_updates.push((bookmark_name.to_owned(), update)),
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        tx_description = format!("push all bookmarkes to git remote {remote}");
    } else if args.tracked {
        for (bookmark_name, targets) in repo.view().local_remote_bookmarkes(&remote) {
            if !targets.remote_ref.is_tracking() {
                continue;
            }
            match classify_bookmark_update(bookmark_name, &remote, targets) {
                Ok(Some(update)) => bookmark_updates.push((bookmark_name.to_owned(), update)),
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        tx_description = format!("push all tracked bookmarkes to git remote {remote}");
    } else if args.deleted {
        for (bookmark_name, targets) in repo.view().local_remote_bookmarkes(&remote) {
            if targets.local_target.is_present() {
                continue;
            }
            match classify_bookmark_update(bookmark_name, &remote, targets) {
                Ok(Some(update)) => bookmark_updates.push((bookmark_name.to_owned(), update)),
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }
        tx_description = format!("push all deleted bookmarkes to git remote {remote}");
    } else {
        let mut seen_bookmarkes: HashSet<&str> = HashSet::new();

        // Process --change bookmarkes first because matching bookmarkes can be moved.
        let change_bookmark_names = update_change_bookmarkes(
            ui,
            &mut tx,
            &args.change,
            &command.settings().push_bookmark_prefix(),
        )?;
        let change_bookmarkes = change_bookmark_names.iter().map(|bookmark_name| {
            let targets = LocalAndRemoteRef {
                local_target: tx.repo().view().get_local_bookmark(bookmark_name),
                remote_ref: tx.repo().view().get_remote_bookmark(bookmark_name, &remote),
            };
            (bookmark_name.as_ref(), targets)
        });
        let bookmarkes_by_name = find_bookmarkes_to_push(repo.view(), &args.bookmark, &remote)?;
        for (bookmark_name, targets) in change_bookmarkes.chain(bookmarkes_by_name.iter().copied())
        {
            if !seen_bookmarkes.insert(bookmark_name) {
                continue;
            }
            match classify_bookmark_update(bookmark_name, &remote, targets) {
                Ok(Some(update)) => bookmark_updates.push((bookmark_name.to_owned(), update)),
                Ok(None) => writeln!(
                    ui.status(),
                    "Branch {bookmark_name}@{remote} already matches {bookmark_name}",
                )?,
                Err(reason) => return Err(reason.into()),
            }
        }

        let use_default_revset =
            args.bookmark.is_empty() && args.change.is_empty() && args.revisions.is_empty();
        let bookmarkes_targeted = find_bookmarkes_targeted_by_revisions(
            ui,
            tx.base_workspace_helper(),
            &remote,
            &args.revisions,
            use_default_revset,
        )?;
        for &(bookmark_name, targets) in &bookmarkes_targeted {
            if !seen_bookmarkes.insert(bookmark_name) {
                continue;
            }
            match classify_bookmark_update(bookmark_name, &remote, targets) {
                Ok(Some(update)) => bookmark_updates.push((bookmark_name.to_owned(), update)),
                Ok(None) => {}
                Err(reason) => reason.print(ui)?,
            }
        }

        tx_description = format!(
            "push {} to git remote {}",
            make_bookmark_term(
                &bookmark_updates
                    .iter()
                    .map(|(bookmark, _)| bookmark.as_str())
                    .collect_vec()
            ),
            &remote
        );
    }
    if bookmark_updates.is_empty() {
        writeln!(ui.status(), "Nothing changed.")?;
        return Ok(());
    }

    let mut bookmark_push_direction = HashMap::new();
    for (bookmark_name, update) in &bookmark_updates {
        let BranchPushUpdate {
            old_target: Some(old_target),
            new_target: Some(new_target),
        } = update
        else {
            continue;
        };
        assert_ne!(old_target, new_target);
        bookmark_push_direction.insert(
            bookmark_name.to_string(),
            if repo.index().is_ancestor(old_target, new_target) {
                BranchMoveDirection::Forward
            } else if repo.index().is_ancestor(new_target, old_target) {
                BranchMoveDirection::Backward
            } else {
                BranchMoveDirection::Sideways
            },
        );
    }

    validate_commits_ready_to_push(&bookmark_updates, &remote, &tx, command, args)?;

    writeln!(ui.status(), "Branch changes to push to {}:", &remote)?;
    for (bookmark_name, update) in &bookmark_updates {
        match (&update.old_target, &update.new_target) {
            (Some(old_target), Some(new_target)) => {
                let old = short_commit_hash(old_target);
                let new = short_commit_hash(new_target);
                // TODO(ilyagr): Add color. Once there is color, "Move bookmark ... sideways"
                // may read more naturally than "Move sideways bookmark ...".
                // Without color, it's hard to see at a glance if one bookmark
                // among many was moved sideways (say). TODO: People on Discord
                // suggest "Move bookmark ... forward by n commits",
                // possibly "Move bookmark ... sideways (X forward, Y back)".
                let msg = match bookmark_push_direction.get(bookmark_name).unwrap() {
                    BranchMoveDirection::Forward => {
                        format!("Move forward bookmark {bookmark_name} from {old} to {new}")
                    }
                    BranchMoveDirection::Backward => {
                        format!("Move backward bookmark {bookmark_name} from {old} to {new}")
                    }
                    BranchMoveDirection::Sideways => {
                        format!("Move sideways bookmark {bookmark_name} from {old} to {new}")
                    }
                };
                writeln!(ui.status(), "  {msg}")?;
            }
            (Some(old_target), None) => {
                writeln!(
                    ui.status(),
                    "  Delete bookmark {bookmark_name} from {}",
                    short_commit_hash(old_target)
                )?;
            }
            (None, Some(new_target)) => {
                writeln!(
                    ui.status(),
                    "  Add bookmark {bookmark_name} to {}",
                    short_commit_hash(new_target)
                )?;
            }
            (None, None) => {
                panic!("Not pushing any change to bookmark {bookmark_name}");
            }
        }
    }

    if args.dry_run {
        writeln!(ui.status(), "Dry-run requested, not pushing.")?;
        return Ok(());
    }

    let targets = GitBranchPushTargets { bookmark_updates };
    let mut writer = GitSidebandProgressMessageWriter::new(ui);
    let mut sideband_progress_callback = |progress_message: &[u8]| {
        _ = writer.write(ui, progress_message);
    };
    with_remote_git_callbacks(ui, Some(&mut sideband_progress_callback), |cb| {
        git::push_bookmarkes(tx.mut_repo(), &git_repo, &remote, &targets, cb)
    })
    .map_err(|err| match err {
        GitPushError::InternalGitError(err) => map_git_error(err),
        GitPushError::RefInUnexpectedLocation(refs) => user_error_with_hint(
            format!(
                "Refusing to push a bookmark that unexpectedly moved on the remote. Affected \
                 refs: {}",
                refs.join(", ")
            ),
            "Try fetching from the remote, then make the bookmark point to where you want it to \
             be, and push again.",
        ),
        _ => user_error(err),
    })?;
    writer.flush(ui)?;
    tx.finish(ui, tx_description)?;
    Ok(())
}

/// Validates that the commits that will be pushed are ready (have authorship
/// information, are not conflicted, etc.)
fn validate_commits_ready_to_push(
    bookmark_updates: &[(String, BranchPushUpdate)],
    remote: &str,
    tx: &WorkspaceCommandTransaction,
    command: &CommandHelper,
    args: &GitPushArgs,
) -> Result<(), CommandError> {
    let workspace_helper = tx.base_workspace_helper();
    let repo = workspace_helper.repo();

    let new_heads = bookmark_updates
        .iter()
        .filter_map(|(_, update)| update.new_target.clone())
        .collect_vec();
    let old_heads = repo
        .view()
        .remote_bookmarkes(remote)
        .flat_map(|(_, old_head)| old_head.target.added_ids())
        .cloned()
        .collect_vec();
    let commits_to_push = RevsetExpression::commits(old_heads)
        .union(&revset_util::parse_immutable_heads_expression(
            &tx.base_workspace_helper().revset_parse_context(),
        )?)
        .range(&RevsetExpression::commits(new_heads));

    let config = command.settings().config();
    let is_private = if let Ok(revset) = config.get_string("git.private-commits") {
        workspace_helper
            .parse_revset(&RevisionArg::from(revset))?
            .evaluate()?
            .containing_fn()
    } else {
        Box::new(|_: &CommitId| false)
    };

    for commit in workspace_helper
        .attach_revset_evaluator(commits_to_push)?
        .evaluate_to_commits()?
    {
        let commit = commit?;
        let mut reasons = vec![];
        if commit.description().is_empty() && !args.allow_empty_description {
            reasons.push("it has no description");
        }
        if commit.author().name.is_empty()
            || commit.author().name == UserSettings::USER_NAME_PLACEHOLDER
            || commit.author().email.is_empty()
            || commit.author().email == UserSettings::USER_EMAIL_PLACEHOLDER
            || commit.committer().name.is_empty()
            || commit.committer().name == UserSettings::USER_NAME_PLACEHOLDER
            || commit.committer().email.is_empty()
            || commit.committer().email == UserSettings::USER_EMAIL_PLACEHOLDER
        {
            reasons.push("it has no author and/or committer set");
        }
        if commit.has_conflict()? {
            reasons.push("it has conflicts");
        }
        if !args.allow_private && is_private(commit.id()) {
            reasons.push("it is private");
        }
        if !reasons.is_empty() {
            return Err(user_error(format!(
                "Won't push commit {} since {}",
                short_commit_hash(commit.id()),
                reasons.join(" and ")
            )));
        }
    }
    Ok(())
}

fn get_default_push_remote(
    ui: &Ui,
    settings: &UserSettings,
    git_repo: &git2::Repository,
) -> Result<String, CommandError> {
    if let Some(remote) = settings.config().get_string("git.push").optional()? {
        Ok(remote)
    } else if let Some(remote) = get_single_remote(git_repo)? {
        // similar to get_default_fetch_remotes
        if remote != DEFAULT_REMOTE {
            writeln!(
                ui.hint_default(),
                "Pushing to the only existing remote: {remote}"
            )?;
        }
        Ok(remote)
    } else {
        Ok(DEFAULT_REMOTE.to_owned())
    }
}

#[derive(Clone, Debug)]
struct RejectedBranchUpdateReason {
    message: String,
    hint: Option<String>,
}

impl RejectedBranchUpdateReason {
    fn print(&self, ui: &Ui) -> io::Result<()> {
        writeln!(ui.warning_default(), "{}", self.message)?;
        if let Some(hint) = &self.hint {
            writeln!(ui.hint_default(), "{hint}")?;
        }
        Ok(())
    }
}

impl From<RejectedBranchUpdateReason> for CommandError {
    fn from(reason: RejectedBranchUpdateReason) -> Self {
        let RejectedBranchUpdateReason { message, hint } = reason;
        let mut cmd_err = user_error(message);
        cmd_err.extend_hints(hint);
        cmd_err
    }
}

fn classify_bookmark_update(
    bookmark_name: &str,
    remote_name: &str,
    targets: LocalAndRemoteRef,
) -> Result<Option<BranchPushUpdate>, RejectedBranchUpdateReason> {
    let push_action = classify_bookmark_push_action(targets);
    match push_action {
        BranchPushAction::AlreadyMatches => Ok(None),
        BranchPushAction::LocalConflicted => Err(RejectedBranchUpdateReason {
            message: format!("Branch {bookmark_name} is conflicted"),
            hint: Some(
                "Run `jj bookmark list` to inspect, and use `jj bookmark set` to fix it up."
                    .to_owned(),
            ),
        }),
        BranchPushAction::RemoteConflicted => Err(RejectedBranchUpdateReason {
            message: format!("Branch {bookmark_name}@{remote_name} is conflicted"),
            hint: Some("Run `jj git fetch` to update the conflicted remote bookmark.".to_owned()),
        }),
        BranchPushAction::RemoteUntracked => Err(RejectedBranchUpdateReason {
            message: format!("Non-tracking remote bookmark {bookmark_name}@{remote_name} exists"),
            hint: Some(format!(
                "Run `jj bookmark track {bookmark_name}@{remote_name}` to import the remote \
                 bookmark."
            )),
        }),
        BranchPushAction::Update(update) => Ok(Some(update)),
    }
}

/// Creates or moves bookmarkes based on the change IDs.
fn update_change_bookmarkes(
    ui: &Ui,
    tx: &mut WorkspaceCommandTransaction,
    changes: &[RevisionArg],
    bookmark_prefix: &str,
) -> Result<Vec<String>, CommandError> {
    if changes.is_empty() {
        // NOTE: we don't want resolve_some_revsets_default_single to fail if the
        // changes argument wasn't provided, so handle that
        return Ok(vec![]);
    }

    let mut bookmark_names = Vec::new();
    let workspace_command = tx.base_workspace_helper();
    let all_commits = workspace_command.resolve_some_revsets_default_single(changes)?;

    for commit in all_commits {
        let workspace_command = tx.base_workspace_helper();
        let short_change_id = short_change_hash(commit.change_id());
        let mut bookmark_name = format!("{bookmark_prefix}{}", commit.change_id().hex());
        let view = tx.base_repo().view();
        if view.get_local_bookmark(&bookmark_name).is_absent() {
            // A local bookmark with the full change ID doesn't exist already, so use the
            // short ID if it's not ambiguous (which it shouldn't be most of the time).
            if workspace_command
                .resolve_single_rev(&RevisionArg::from(short_change_id.clone()))
                .is_ok()
            {
                // Short change ID is not ambiguous, so update the bookmark name to use it.
                bookmark_name = format!("{bookmark_prefix}{short_change_id}");
            };
        }
        if view.get_local_bookmark(&bookmark_name).is_absent() {
            writeln!(
                ui.status(),
                "Creating bookmark {bookmark_name} for revision {short_change_id}",
            )?;
        }
        tx.mut_repo()
            .set_local_bookmark_target(&bookmark_name, RefTarget::normal(commit.id().clone()));
        bookmark_names.push(bookmark_name);
    }
    Ok(bookmark_names)
}

fn find_bookmarkes_to_push<'a>(
    view: &'a View,
    bookmark_patterns: &[StringPattern],
    remote_name: &str,
) -> Result<Vec<(&'a str, LocalAndRemoteRef<'a>)>, CommandError> {
    let mut matching_bookmarkes = vec![];
    let mut unmatched_patterns = vec![];
    for pattern in bookmark_patterns {
        let mut matches = view
            .local_remote_bookmarkes_matching(pattern, remote_name)
            .filter(|(_, targets)| {
                // If the remote exists but is not tracking, the absent local shouldn't
                // be considered a deleted bookmark.
                targets.local_target.is_present() || targets.remote_ref.is_tracking()
            })
            .peekable();
        if matches.peek().is_none() {
            unmatched_patterns.push(pattern);
        }
        matching_bookmarkes.extend(matches);
    }
    match &unmatched_patterns[..] {
        [] => Ok(matching_bookmarkes),
        [pattern] if pattern.is_exact() => Err(user_error(format!("No such bookmark: {pattern}"))),
        patterns => Err(user_error(format!(
            "No matching bookmarkes for patterns: {}",
            patterns.iter().join(", ")
        ))),
    }
}

fn find_bookmarkes_targeted_by_revisions<'a>(
    ui: &Ui,
    workspace_command: &'a WorkspaceCommandHelper,
    remote_name: &str,
    revisions: &[RevisionArg],
    use_default_revset: bool,
) -> Result<Vec<(&'a str, LocalAndRemoteRef<'a>)>, CommandError> {
    let mut revision_commit_ids = HashSet::new();
    if use_default_revset {
        let Some(wc_commit_id) = workspace_command.get_wc_commit_id().cloned() else {
            return Err(user_error("Nothing checked out in this workspace"));
        };
        let current_bookmarkes_expression = RevsetExpression::remote_bookmarkes(
            StringPattern::everything(),
            StringPattern::exact(remote_name),
            None,
        )
        .range(&RevsetExpression::commit(wc_commit_id))
        .intersection(&RevsetExpression::bookmarkes(StringPattern::everything()));
        let current_bookmarkes_revset = current_bookmarkes_expression
            .evaluate_programmatic(workspace_command.repo().as_ref())?;
        revision_commit_ids.extend(current_bookmarkes_revset.iter());
        if revision_commit_ids.is_empty() {
            writeln!(
                ui.warning_default(),
                "No bookmarkes found in the default push revset: \
                 remote_bookmarkes(remote={remote_name})..@"
            )?;
        }
    }
    for rev_arg in revisions {
        let mut expression = workspace_command.parse_revset(rev_arg)?;
        expression.intersect_with(&RevsetExpression::bookmarkes(StringPattern::everything()));
        let mut commit_ids = expression.evaluate_to_commit_ids()?.peekable();
        if commit_ids.peek().is_none() {
            writeln!(
                ui.warning_default(),
                "No bookmarkes point to the specified revisions: {rev_arg}"
            )?;
        }
        revision_commit_ids.extend(commit_ids);
    }
    let bookmarkes_targeted = workspace_command
        .repo()
        .view()
        .local_remote_bookmarkes(remote_name)
        .filter(|(_, targets)| {
            let mut local_ids = targets.local_target.added_ids();
            local_ids.any(|id| revision_commit_ids.contains(id))
        })
        .collect_vec();
    Ok(bookmarkes_targeted)
}
