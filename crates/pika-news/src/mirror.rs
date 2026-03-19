use std::env;

use anyhow::Context;

use crate::branch_store::{MirrorStatusRecord, MirrorSyncRunInput, MirrorSyncRunRecord};
use crate::config::Config;
use crate::forge;
use crate::storage::Store;

#[derive(Debug, Default)]
pub struct MirrorPassResult {
    pub attempted: bool,
    pub status: Option<String>,
    pub lagging_ref_count: Option<i64>,
}

pub fn run_mirror_pass(
    store: &Store,
    config: &Config,
    trigger_source: &str,
) -> anyhow::Result<MirrorPassResult> {
    let Some(forge_repo) = config.effective_forge_repo() else {
        return Ok(MirrorPassResult::default());
    };
    let Some(remote_name) = forge_repo.mirror_remote.as_deref() else {
        return Ok(MirrorPassResult::default());
    };
    let github_token = env::var(&config.github_token_env).ok();
    match forge::sync_mirror(&forge_repo, remote_name, github_token.as_deref()) {
        Ok(outcome) => {
            store.record_mirror_sync_run(&MirrorSyncRunInput {
                repo: forge_repo.repo.clone(),
                canonical_git_dir: forge_repo.canonical_git_dir.clone(),
                default_branch: forge_repo.default_branch.clone(),
                remote_name: outcome.remote_name.clone(),
                trigger_source: trigger_source.to_string(),
                status: "success".to_string(),
                local_default_head: outcome.local_default_head.clone(),
                remote_default_head: outcome.remote_default_head.clone(),
                lagging_ref_count: Some(outcome.lagging_ref_count),
                synced_ref_count: Some(outcome.synced_ref_count),
                error_text: None,
            })?;
            Ok(MirrorPassResult {
                attempted: true,
                status: Some("success".to_string()),
                lagging_ref_count: Some(outcome.lagging_ref_count),
            })
        }
        Err(err) => {
            let inspected = forge::inspect_mirror(&forge_repo, remote_name).ok();
            store.record_mirror_sync_run(&MirrorSyncRunInput {
                repo: forge_repo.repo.clone(),
                canonical_git_dir: forge_repo.canonical_git_dir.clone(),
                default_branch: forge_repo.default_branch.clone(),
                remote_name: remote_name.to_string(),
                trigger_source: trigger_source.to_string(),
                status: "failed".to_string(),
                local_default_head: inspected
                    .as_ref()
                    .and_then(|state| state.local_default_head.clone()),
                remote_default_head: inspected
                    .as_ref()
                    .and_then(|state| state.remote_default_head.clone()),
                lagging_ref_count: inspected.as_ref().map(|state| state.lagging_ref_count),
                synced_ref_count: inspected.as_ref().map(|state| state.synced_ref_count),
                error_text: Some(err.to_string()),
            })?;
            Ok(MirrorPassResult {
                attempted: true,
                status: Some("failed".to_string()),
                lagging_ref_count: inspected.as_ref().map(|state| state.lagging_ref_count),
            })
        }
    }
}

pub fn get_mirror_status(
    store: &Store,
    config: &Config,
) -> anyhow::Result<Option<(MirrorStatusRecord, Vec<MirrorSyncRunRecord>)>> {
    let Some(forge_repo) = config.effective_forge_repo() else {
        return Ok(None);
    };
    let Some(remote_name) = forge_repo.mirror_remote.as_deref() else {
        return Ok(None);
    };
    let status = store
        .get_mirror_status(&forge_repo.repo, remote_name)
        .context("load mirror status")?;
    let Some(status) = status else {
        return Ok(None);
    };
    let history = store
        .list_recent_mirror_sync_runs(&forge_repo.repo, remote_name, 12)
        .context("load mirror history")?;
    Ok(Some((status, history)))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::run_mirror_pass;
    use crate::config::{Config, ForgeRepoConfig};
    use crate::storage::Store;

    fn git<P: AsRef<std::path::Path>>(cwd: P, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd.as_ref())
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn base_config(canonical_git_dir: &str, mirror_remote: Option<&str>) -> Config {
        Config {
            repos: vec!["sledtools/pika".to_string()],
            forge_repo: Some(ForgeRepoConfig {
                repo: "sledtools/pika".to_string(),
                canonical_git_dir: canonical_git_dir.to_string(),
                default_branch: "master".to_string(),
                mirror_remote: mirror_remote.map(str::to_string),
                ci_command: vec!["just".to_string(), "pre-merge".to_string()],
                hook_url: Some("http://127.0.0.1:9999/news/webhook".to_string()),
            }),
            poll_interval_secs: 60,
            model: "test-model".to_string(),
            api_key_env: "ANTHROPIC_API_KEY".to_string(),
            github_token_env: "GITHUB_TOKEN".to_string(),
            merged_lookback_hours: 72,
            worker_concurrency: 1,
            retry_backoff_secs: 120,
            webhook_secret_env: "PIKA_NEWS_WEBHOOK_SECRET".to_string(),
            bind_address: "127.0.0.1".to_string(),
            bind_port: 8787,
            allowed_npubs: vec![],
            bootstrap_admin_npubs: vec![],
        }
    }

    #[test]
    fn mirror_pass_records_success_and_zero_lag() {
        let root = tempfile::tempdir().expect("create temp root");
        let canonical = root.path().join("canonical.git");
        let mirror = root.path().join("mirror.git");
        let seed = root.path().join("seed");
        git(
            root.path(),
            &["init", "--bare", canonical.to_str().unwrap()],
        );
        git(root.path(), &["init", "--bare", mirror.to_str().unwrap()]);
        git(root.path(), &["init", seed.to_str().unwrap()]);
        git(&seed, &["config", "user.name", "Test User"]);
        git(&seed, &["config", "user.email", "test@example.com"]);
        std::fs::write(seed.join("README.md"), "hello\n").unwrap();
        git(&seed, &["add", "README.md"]);
        git(&seed, &["commit", "-m", "initial"]);
        git(&seed, &["branch", "-M", "master"]);
        git(
            &seed,
            &["remote", "add", "origin", canonical.to_str().unwrap()],
        );
        git(&seed, &["push", "origin", "master"]);
        Command::new("git")
            .args([
                "--git-dir",
                canonical.to_str().unwrap(),
                "remote",
                "add",
                "github",
                mirror.to_str().unwrap(),
            ])
            .status()
            .expect("add mirror remote");

        let store = Store::open(&root.path().join("pika-news.db")).expect("open store");
        let result = run_mirror_pass(
            &store,
            &base_config(canonical.to_str().unwrap(), Some("github")),
            "background",
        )
        .expect("run mirror pass");
        assert!(result.attempted);
        assert_eq!(result.status.as_deref(), Some("success"));
        assert_eq!(result.lagging_ref_count, Some(0));

        let status = store
            .get_mirror_status("sledtools/pika", "github")
            .expect("mirror status")
            .expect("status exists");
        assert_eq!(
            status.last_attempt.as_ref().map(|run| run.status.as_str()),
            Some("success")
        );
    }

    #[test]
    fn mirror_pass_records_failure_for_bad_remote() {
        let root = tempfile::tempdir().expect("create temp root");
        let canonical = root.path().join("canonical.git");
        git(
            root.path(),
            &["init", "--bare", canonical.to_str().unwrap()],
        );
        let store = Store::open(&root.path().join("pika-news.db")).expect("open store");
        let result = run_mirror_pass(
            &store,
            &base_config(canonical.to_str().unwrap(), Some("github")),
            "manual",
        )
        .expect("run failed mirror pass");
        assert!(result.attempted);
        assert_eq!(result.status.as_deref(), Some("failed"));

        let status = store
            .get_mirror_status("sledtools/pika", "github")
            .expect("mirror status")
            .expect("status exists");
        let attempt = status.last_attempt.expect("attempt");
        assert_eq!(attempt.status, "failed");
        assert!(attempt
            .error_text
            .unwrap_or_default()
            .contains("mirror remote"));
    }
}
