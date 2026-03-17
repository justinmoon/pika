use anyhow::Context;

use crate::config::Config;
use crate::forge;
use crate::storage::Store;

#[derive(Debug, Default)]
pub struct CiPassResult {
    pub claimed: usize,
    pub succeeded: usize,
    pub failed: usize,
}

pub fn run_ci_pass(store: &Store, config: &Config) -> anyhow::Result<CiPassResult> {
    let Some(forge_repo) = config.effective_forge_repo() else {
        return Ok(CiPassResult::default());
    };

    let runs = store
        .claim_pending_ci_runs(1)
        .context("claim pending ci runs")?;
    if runs.is_empty() {
        return Ok(CiPassResult::default());
    }

    let mut result = CiPassResult {
        claimed: runs.len(),
        ..CiPassResult::default()
    };
    for run in runs {
        let exec = forge::run_ci_command_for_head(&forge_repo, &run.source_head_sha, &run.command)
            .with_context(|| format!("run ci for branch {}", run.branch_id));
        match exec {
            Ok(exec) => {
                let status = if exec.success { "success" } else { "failed" };
                store
                    .finish_ci_run(run.run_id, status, &exec.log)
                    .with_context(|| format!("persist ci result for run {}", run.run_id))?;
                if exec.success {
                    result.succeeded += 1;
                } else {
                    result.failed += 1;
                }
            }
            Err(err) => {
                let log = format!("ci runner error: {}", err);
                store
                    .finish_ci_run(run.run_id, "failed", &log)
                    .with_context(|| format!("persist ci failure for run {}", run.run_id))?;
                result.failed += 1;
            }
        }
    }

    Ok(result)
}
