use std::thread;

use anyhow::Context;

use crate::branch_store::BranchGenerationJob;
use crate::config::Config;
use crate::forge;
use crate::model::{self, GenerationError, PromptInput, PromptPrMetadata};
use crate::render;
use crate::storage::Store;
use crate::tutorial;

#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct WorkerPassResult {
    pub claimed: usize,
    pub ready: usize,
    pub failed: usize,
    pub retry_scheduled: usize,
}

pub fn run_generation_pass(store: &Store, config: &Config) -> anyhow::Result<WorkerPassResult> {
    let Some(forge_repo) = config.effective_forge_repo() else {
        return Ok(WorkerPassResult::default());
    };

    let jobs = store
        .claim_pending_branch_generation_jobs(config.worker_concurrency)
        .context("claim pending branch generation jobs")?;
    if jobs.is_empty() {
        return Ok(WorkerPassResult::default());
    }

    let mut handles = Vec::with_capacity(jobs.len());
    for job in jobs {
        let store = store.clone();
        let forge_repo = forge_repo.clone();
        let model = config.model.clone();
        let api_key_env = config.api_key_env.clone();
        let retry_backoff_secs = config.retry_backoff_secs;

        handles.push(thread::spawn(move || {
            process_job(
                &store,
                &forge_repo,
                &job,
                &model,
                &api_key_env,
                retry_backoff_secs,
            )
        }));
    }

    let mut result = WorkerPassResult {
        claimed: handles.len(),
        ..WorkerPassResult::default()
    };
    for handle in handles {
        match handle.join() {
            Ok(Ok(JobOutcome::Ready)) => result.ready += 1,
            Ok(Ok(JobOutcome::RetryScheduled)) => result.retry_scheduled += 1,
            Ok(Ok(JobOutcome::Failed)) => result.failed += 1,
            Ok(Err(err)) => {
                result.failed += 1;
                eprintln!("worker thread returned error: {}", err);
            }
            Err(_) => {
                result.failed += 1;
                eprintln!("worker thread panicked");
            }
        }
    }

    Ok(result)
}

enum JobOutcome {
    Ready,
    RetryScheduled,
    Failed,
}

fn process_job(
    store: &Store,
    forge_repo: &crate::config::ForgeRepoConfig,
    job: &BranchGenerationJob,
    model_name: &str,
    api_key_env: &str,
    retry_backoff_secs: u64,
) -> anyhow::Result<JobOutcome> {
    let diff = forge::branch_diff(forge_repo, &job.merge_base_sha, &job.head_sha)
        .with_context(|| format!("collect branch diff for {}", job.branch_name));
    let diff = match diff {
        Ok(diff) => diff,
        Err(err) => {
            store
                .mark_branch_generation_failed(
                    job.artifact_id,
                    &format!("diff fetch failed: {}", err),
                    true,
                    retry_backoff_secs,
                )
                .context("persist diff-fetch failure")?;
            return Ok(JobOutcome::RetryScheduled);
        }
    };

    let prompt_input = PromptInput {
        pr: PromptPrMetadata {
            repo: job.repo.clone(),
            number: Some(job.branch_id as u64),
            title: format!("branch {}: {}", job.branch_name, job.title),
            head_sha: Some(job.head_sha.clone()),
            base_ref: forge_repo.default_branch.clone(),
        },
        files: tutorial::extract_files(&diff),
        unified_diff: model::bounded_diff(&diff, 60_000),
    };

    let generated = model::generate_with_anthropic(model_name, api_key_env, &prompt_input);
    let gen_output = match generated {
        Ok(output) => output,
        Err(GenerationError::RetrySafe(message)) => {
            store
                .mark_branch_generation_failed(job.artifact_id, &message, true, retry_backoff_secs)
                .with_context(|| {
                    format!(
                        "persist retry-safe failure for artifact {}",
                        job.artifact_id
                    )
                })?;
            return Ok(JobOutcome::RetryScheduled);
        }
        Err(GenerationError::MissingApiKey { env_var }) => {
            store
                .mark_branch_generation_failed(
                    job.artifact_id,
                    &format!("missing API key env var {}", env_var),
                    false,
                    retry_backoff_secs,
                )
                .with_context(|| {
                    format!(
                        "persist missing-key failure for artifact {}",
                        job.artifact_id
                    )
                })?;
            return Ok(JobOutcome::Failed);
        }
        Err(GenerationError::Fatal(message)) => {
            store
                .mark_branch_generation_failed(job.artifact_id, &message, false, retry_backoff_secs)
                .with_context(|| {
                    format!("persist fatal failure for artifact {}", job.artifact_id)
                })?;
            return Ok(JobOutcome::Failed);
        }
    };

    let html = render::render_tutorial_html(
        &format!("branch #{} {}", job.branch_id, job.branch_name),
        &forge_repo.default_branch,
        &diff,
        &gen_output.tutorial,
    );
    let tutorial_json =
        serde_json::to_string(&gen_output.tutorial).context("serialize tutorial JSON")?;
    store
        .mark_branch_generation_ready(job.artifact_id, &tutorial_json, &html, &job.head_sha, &diff)
        .with_context(|| format!("mark branch artifact {} ready", job.artifact_id))?;

    Ok(JobOutcome::Ready)
}
