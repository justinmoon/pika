use anyhow::{anyhow, Context};

use crate::branch_store::BranchUpsertInput;
use crate::config::Config;
use crate::forge;
use crate::storage::Store;

#[derive(Debug, Default)]
pub struct PollResult {
    pub repos_polled: usize,
    pub branches_seen: usize,
    pub queued_regenerations: usize,
    pub head_sha_changes: usize,
    pub stale_closed: usize,
    pub queued_ci_runs: usize,
}

pub fn poll_once_limited(
    store: &Store,
    config: &Config,
    max_branches: usize,
) -> anyhow::Result<PollResult> {
    let forge_repo = config
        .effective_forge_repo()
        .ok_or_else(|| anyhow!("forge_repo must be configured for hosted forge mode"))?;

    let ci_entrypoint = forge_repo.ci_command.join(" ");
    let ci_command_json =
        serde_json::to_string(&forge_repo.ci_command).context("serialize configured ci command")?;
    let branches = forge::list_branches(&forge_repo).with_context(|| {
        format!(
            "list canonical branches from {}",
            forge_repo.canonical_git_dir
        )
    })?;

    let mut result = PollResult::default();
    let present_names: Vec<String> = branches
        .iter()
        .map(|branch| branch.branch_name.clone())
        .collect();
    for branch in branches.into_iter().take(if max_branches == 0 {
        usize::MAX
    } else {
        max_branches
    }) {
        let outcome = store
            .upsert_branch_record(&BranchUpsertInput {
                repo: forge_repo.repo.clone(),
                canonical_git_dir: forge_repo.canonical_git_dir.clone(),
                default_branch: forge_repo.default_branch.clone(),
                ci_entrypoint: ci_entrypoint.clone(),
                ci_command_json: ci_command_json.clone(),
                branch_name: branch.branch_name,
                title: branch.title,
                head_sha: branch.head_sha,
                merge_base_sha: branch.merge_base_sha,
                author_name: branch.author_name,
                author_email: branch.author_email,
                updated_at: branch.updated_at,
            })
            .context("upsert branch record from canonical repo state")?;
        result.branches_seen += 1;
        if outcome.head_changed {
            result.head_sha_changes += 1;
        }
        if outcome.queued_generation {
            result.queued_regenerations += 1;
        }
        if outcome.queued_ci {
            result.queued_ci_runs += 1;
        }
    }

    result.stale_closed = store
        .close_missing_open_branches(&forge_repo.repo, &present_names)
        .with_context(|| format!("close stale open branches for {}", forge_repo.repo))?;
    result.repos_polled = 1;
    Ok(result)
}
