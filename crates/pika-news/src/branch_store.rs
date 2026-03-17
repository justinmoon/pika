use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::storage::Store;

#[derive(Debug, Clone)]
pub struct BranchUpsertInput {
    pub repo: String,
    pub canonical_git_dir: String,
    pub default_branch: String,
    pub ci_entrypoint: String,
    pub ci_command_json: String,
    pub branch_name: String,
    pub title: String,
    pub head_sha: String,
    pub merge_base_sha: String,
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct BranchUpsertOutcome {
    pub branch_id: i64,
    pub inserted: bool,
    pub head_changed: bool,
    pub queued_generation: bool,
    pub queued_ci: bool,
}

#[derive(Debug, Clone)]
pub struct BranchGenerationJob {
    pub artifact_id: i64,
    pub branch_id: i64,
    pub repo: String,
    pub branch_name: String,
    pub title: String,
    pub head_sha: String,
    pub merge_base_sha: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BranchFeedItem {
    pub branch_id: i64,
    pub repo: String,
    pub branch_name: String,
    pub title: String,
    pub state: String,
    pub updated_at: String,
    pub head_sha: String,
    pub tutorial_status: String,
    pub ci_status: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BranchDetailRecord {
    pub branch_id: i64,
    pub repo: String,
    pub branch_name: String,
    pub title: String,
    pub branch_state: String,
    pub updated_at: String,
    pub target_branch: String,
    pub head_sha: String,
    pub merge_base_sha: String,
    pub merge_commit_sha: Option<String>,
    pub tutorial_status: String,
    pub tutorial_json: Option<String>,
    pub unified_diff: Option<String>,
    pub error_message: Option<String>,
    pub ci_status: String,
}

#[derive(Debug, Clone)]
pub struct BranchCiRunRecord {
    pub id: i64,
    pub source_head_sha: String,
    pub entrypoint: String,
    pub status: String,
    pub log_text: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PendingCiRun {
    pub run_id: i64,
    pub branch_id: i64,
    pub source_head_sha: String,
    pub entrypoint: String,
    pub command: Vec<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BranchActionTarget {
    pub branch_id: i64,
    pub repo: String,
    pub branch_name: String,
    pub title: String,
    pub branch_state: String,
    pub target_branch: String,
    pub head_sha: String,
    pub merge_base_sha: String,
}

impl Store {
    pub fn upsert_branch_record(
        &self,
        input: &BranchUpsertInput,
    ) -> anyhow::Result<BranchUpsertOutcome> {
        self.with_connection(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .context("start branch upsert transaction")?;

            let repo_id = ensure_repo_metadata(
                &tx,
                &input.repo,
                &input.canonical_git_dir,
                &input.default_branch,
                &input.ci_entrypoint,
            )?;
            let existing: Option<(i64, String)> = tx
                .query_row(
                    "SELECT id, head_sha
                     FROM branch_records
                     WHERE repo_id = ?1 AND branch_name = ?2 AND state = 'open'
                     ORDER BY id DESC
                     LIMIT 1",
                    params![repo_id, input.branch_name],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .context("lookup existing open branch record")?;

            let (branch_id, inserted, previous_head_sha) = match existing {
                Some((branch_id, previous_head_sha)) => {
                    tx.execute(
                        "UPDATE branch_records
                         SET title = ?1,
                             target_branch = ?2,
                             head_sha = ?3,
                             merge_base_sha = ?4,
                             author_name = ?5,
                             author_email = ?6,
                             updated_at = ?7,
                             last_seen_at = CURRENT_TIMESTAMP,
                             deleted_at = NULL
                         WHERE id = ?8",
                        params![
                            input.title,
                            input.default_branch,
                            input.head_sha,
                            input.merge_base_sha,
                            input.author_name,
                            input.author_email,
                            input.updated_at,
                            branch_id,
                        ],
                    )
                    .context("update existing branch record")?;
                    (branch_id, false, Some(previous_head_sha))
                }
                None => {
                    tx.execute(
                        "INSERT INTO branch_records(
                            repo_id,
                            branch_name,
                            target_branch,
                            title,
                            state,
                            head_sha,
                            merge_base_sha,
                            author_name,
                            author_email,
                            updated_at,
                            opened_at,
                            last_seen_at
                         ) VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6, ?7, ?8, ?9, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
                        params![
                            repo_id,
                            input.branch_name,
                            input.default_branch,
                            input.title,
                            input.head_sha,
                            input.merge_base_sha,
                            input.author_name,
                            input.author_email,
                            input.updated_at,
                        ],
                    )
                    .context("insert branch record")?;
                    (tx.last_insert_rowid(), true, None)
                }
            };

            let head_changed = previous_head_sha
                .as_deref()
                .map(|previous| previous != input.head_sha)
                .unwrap_or(true);
            let queued_generation =
                ensure_branch_artifact_for_head(&tx, branch_id, &input.head_sha, &input.merge_base_sha)?;
            let queued_ci = ensure_ci_run_for_head(
                &tx,
                branch_id,
                &input.head_sha,
                &input.ci_entrypoint,
                &input.ci_command_json,
            )?;

            tx.commit().context("commit branch upsert transaction")?;
            Ok(BranchUpsertOutcome {
                branch_id,
                inserted,
                head_changed,
                queued_generation,
                queued_ci,
            })
        })
    }

    pub fn close_missing_open_branches(
        &self,
        repo: &str,
        present_branch_names: &[String],
    ) -> anyhow::Result<usize> {
        self.with_connection(|conn| {
            let repo_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM repos WHERE repo = ?1",
                    params![repo],
                    |row| row.get(0),
                )
                .optional()
                .context("lookup repo for stale branch close")?;
            let Some(repo_id) = repo_id else {
                return Ok(0);
            };

            let mut sql = String::from(
                "UPDATE branch_records
                 SET state = 'closed',
                     closed_at = COALESCE(closed_at, CURRENT_TIMESTAMP),
                     deleted_at = COALESCE(deleted_at, CURRENT_TIMESTAMP),
                     updated_at = CURRENT_TIMESTAMP
                 WHERE repo_id = ?1
                   AND state = 'open'",
            );
            if !present_branch_names.is_empty() {
                let placeholders = (0..present_branch_names.len())
                    .map(|idx| format!("?{}", idx + 2))
                    .collect::<Vec<_>>()
                    .join(", ");
                sql.push_str(&format!(" AND branch_name NOT IN ({placeholders})"));
            }
            let mut stmt = conn
                .prepare(&sql)
                .context("prepare close stale branch query")?;
            let mut params_dyn: Vec<&dyn rusqlite::ToSql> =
                Vec::with_capacity(present_branch_names.len() + 1);
            params_dyn.push(&repo_id);
            for name in present_branch_names {
                params_dyn.push(name);
            }
            let changed = stmt
                .execute(params_dyn.as_slice())
                .context("close stale branch records")?;
            Ok(changed)
        })
    }

    pub fn list_branch_feed_items(&self) -> anyhow::Result<Vec<BranchFeedItem>> {
        self.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT br.id,
                            r.repo,
                            br.branch_name,
                            br.title,
                            br.state,
                            br.updated_at,
                            br.head_sha,
                            COALESCE(ba.status, 'pending'),
                            COALESCE(ci.status, 'queued')
                     FROM branch_records br
                     JOIN repos r ON r.id = br.repo_id
                     LEFT JOIN branch_artifact_versions ba ON ba.id = (
                        SELECT bav.id
                        FROM branch_artifact_versions bav
                        WHERE bav.branch_id = br.id AND bav.status != 'superseded'
                        ORDER BY bav.version DESC
                        LIMIT 1
                     )
                     LEFT JOIN branch_ci_runs ci ON ci.id = (
                        SELECT bcr.id
                        FROM branch_ci_runs bcr
                        WHERE bcr.branch_id = br.id
                        ORDER BY bcr.id DESC
                        LIMIT 1
                     )
                     WHERE br.state IN ('open', 'merged', 'closed')
                     ORDER BY CASE br.state WHEN 'open' THEN 0 ELSE 1 END, br.updated_at DESC, br.id DESC",
                )
                .context("prepare branch feed query")?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(BranchFeedItem {
                        branch_id: row.get(0)?,
                        repo: row.get(1)?,
                        branch_name: row.get(2)?,
                        title: row.get(3)?,
                        state: row.get(4)?,
                        updated_at: row.get(5)?,
                        head_sha: row.get(6)?,
                        tutorial_status: row.get(7)?,
                        ci_status: row.get(8)?,
                    })
                })
                .context("query branch feed items")?;

            let mut items = Vec::new();
            for row in rows {
                items.push(row.context("read branch feed row")?);
            }
            Ok(items)
        })
    }

    pub fn get_branch_detail(&self, branch_id: i64) -> anyhow::Result<Option<BranchDetailRecord>> {
        self.with_connection(|conn| {
            conn.query_row(
                "SELECT br.id,
                        r.repo,
                        br.branch_name,
                        br.title,
                        br.state,
                        br.updated_at,
                        br.target_branch,
                        br.head_sha,
                        br.merge_base_sha,
                        br.merge_commit_sha,
                        COALESCE(ba_latest.status, 'pending'),
                        ba_current.tutorial_json,
                        ba_current.unified_diff,
                        ba_latest.error_message,
                        COALESCE(ci.status, 'queued')
                 FROM branch_records br
                 JOIN repos r ON r.id = br.repo_id
                 LEFT JOIN branch_artifact_versions ba_latest ON ba_latest.id = (
                    SELECT bav.id
                    FROM branch_artifact_versions bav
                    WHERE bav.branch_id = br.id AND bav.status != 'superseded'
                    ORDER BY bav.version DESC
                    LIMIT 1
                 )
                 LEFT JOIN branch_artifact_versions ba_current ON ba_current.id = (
                    SELECT bav.id
                    FROM branch_artifact_versions bav
                    WHERE bav.branch_id = br.id AND bav.is_current = 1 AND bav.status = 'ready'
                    ORDER BY bav.version DESC
                    LIMIT 1
                 )
                 LEFT JOIN branch_ci_runs ci ON ci.id = (
                    SELECT bcr.id
                    FROM branch_ci_runs bcr
                    WHERE bcr.branch_id = br.id
                    ORDER BY bcr.id DESC
                    LIMIT 1
                 )
                 WHERE br.id = ?1",
                params![branch_id],
                |row| {
                    Ok(BranchDetailRecord {
                        branch_id: row.get(0)?,
                        repo: row.get(1)?,
                        branch_name: row.get(2)?,
                        title: row.get(3)?,
                        branch_state: row.get(4)?,
                        updated_at: row.get(5)?,
                        target_branch: row.get(6)?,
                        head_sha: row.get(7)?,
                        merge_base_sha: row.get(8)?,
                        merge_commit_sha: row.get(9)?,
                        tutorial_status: row.get(10)?,
                        tutorial_json: row.get(11)?,
                        unified_diff: row.get(12)?,
                        error_message: row.get(13)?,
                        ci_status: row.get(14)?,
                    })
                },
            )
            .optional()
            .context("query branch detail")
        })
    }

    pub fn list_branch_ci_runs(
        &self,
        branch_id: i64,
        limit: usize,
    ) -> anyhow::Result<Vec<BranchCiRunRecord>> {
        self.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, source_head_sha, entrypoint, status, log_text, created_at, started_at, finished_at
                     FROM branch_ci_runs
                     WHERE branch_id = ?1
                     ORDER BY id DESC
                     LIMIT ?2",
                )
                .context("prepare branch ci runs query")?;
            let rows = stmt
                .query_map(params![branch_id, limit as i64], |row| {
                    Ok(BranchCiRunRecord {
                        id: row.get(0)?,
                        source_head_sha: row.get(1)?,
                        entrypoint: row.get(2)?,
                        status: row.get(3)?,
                        log_text: row.get(4)?,
                        created_at: row.get(5)?,
                        started_at: row.get(6)?,
                        finished_at: row.get(7)?,
                    })
                })
                .context("query branch ci runs")?;

            let mut runs = Vec::new();
            for row in rows {
                runs.push(row.context("read branch ci run row")?);
            }
            Ok(runs)
        })
    }

    pub fn get_branch_action_target(
        &self,
        branch_id: i64,
    ) -> anyhow::Result<Option<BranchActionTarget>> {
        self.with_connection(|conn| {
            conn.query_row(
                "SELECT br.id, r.repo, br.branch_name, br.title, br.state, br.target_branch, br.head_sha, br.merge_base_sha
                 FROM branch_records br
                 JOIN repos r ON r.id = br.repo_id
                 WHERE br.id = ?1",
                params![branch_id],
                |row| {
                    Ok(BranchActionTarget {
                        branch_id: row.get(0)?,
                        repo: row.get(1)?,
                        branch_name: row.get(2)?,
                        title: row.get(3)?,
                        branch_state: row.get(4)?,
                        target_branch: row.get(5)?,
                        head_sha: row.get(6)?,
                        merge_base_sha: row.get(7)?,
                    })
                },
            )
            .optional()
            .context("query branch action target")
        })
    }

    pub fn mark_branch_merged(
        &self,
        branch_id: i64,
        merged_by: &str,
        merge_commit_sha: &str,
    ) -> anyhow::Result<()> {
        self.with_connection(|conn| {
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .context("start mark branch merged transaction")?;
            let branch: BranchActionTarget = tx
                .query_row(
                    "SELECT br.id, r.repo, br.branch_name, br.title, br.state, br.target_branch, br.head_sha, br.merge_base_sha
                     FROM branch_records br
                     JOIN repos r ON r.id = br.repo_id
                     WHERE br.id = ?1",
                    params![branch_id],
                    |row| {
                        Ok(BranchActionTarget {
                            branch_id: row.get(0)?,
                            repo: row.get(1)?,
                            branch_name: row.get(2)?,
                            title: row.get(3)?,
                            branch_state: row.get(4)?,
                            target_branch: row.get(5)?,
                            head_sha: row.get(6)?,
                            merge_base_sha: row.get(7)?,
                        })
                    },
                )
                .context("lookup branch for merge record")?;
            tx.execute(
                "UPDATE branch_records
                 SET state = 'merged',
                     merge_commit_sha = ?1,
                     merged_by = ?2,
                     merged_at = CURRENT_TIMESTAMP,
                     closed_at = CURRENT_TIMESTAMP,
                     deleted_at = CURRENT_TIMESTAMP,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?3",
                params![merge_commit_sha, merged_by, branch_id],
            )
            .context("update branch merged state")?;
            tx.execute(
                "INSERT INTO merge_records(
                    branch_id,
                    repo,
                    branch_name,
                    target_branch,
                    head_sha,
                    merge_base_sha,
                    merge_commit_sha,
                    merged_by
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    branch.branch_id,
                    branch.repo,
                    branch.branch_name,
                    branch.target_branch,
                    branch.head_sha,
                    branch.merge_base_sha,
                    merge_commit_sha,
                    merged_by,
                ],
            )
            .context("insert merge record")?;
            tx.commit().context("commit merged branch transaction")?;
            Ok(())
        })
    }

    pub fn mark_branch_closed(&self, branch_id: i64, closed_by: &str) -> anyhow::Result<()> {
        self.with_connection(|conn| {
            conn.execute(
                "UPDATE branch_records
                 SET state = 'closed',
                     closed_by = ?1,
                     closed_at = CURRENT_TIMESTAMP,
                     deleted_at = COALESCE(deleted_at, CURRENT_TIMESTAMP),
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?2",
                params![closed_by, branch_id],
            )
            .context("update branch closed state")?;
            Ok(())
        })
    }

    pub fn claim_pending_branch_generation_jobs(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<BranchGenerationJob>> {
        self.with_connection(|conn| {
            let tx = conn
                .unchecked_transaction()
                .context("start branch generation claim transaction")?;
            let mut jobs = Vec::new();
            {
                let mut stmt = tx
                    .prepare(
                        "SELECT bav.id, br.id, r.repo, br.branch_name, br.title, bav.source_head_sha, bav.merge_base_sha
                         FROM branch_artifact_versions bav
                         JOIN branch_records br ON br.id = bav.branch_id
                         JOIN repos r ON r.id = br.repo_id
                         WHERE bav.status = 'pending'
                           AND (bav.next_retry_at IS NULL OR bav.next_retry_at <= CURRENT_TIMESTAMP)
                         ORDER BY bav.updated_at ASC
                         LIMIT ?1",
                    )
                    .context("prepare branch generation claim query")?;
                let rows = stmt
                    .query_map(params![limit as i64], |row| {
                        Ok(BranchGenerationJob {
                            artifact_id: row.get(0)?,
                            branch_id: row.get(1)?,
                            repo: row.get(2)?,
                            branch_name: row.get(3)?,
                            title: row.get(4)?,
                            head_sha: row.get(5)?,
                            merge_base_sha: row.get(6)?,
                        })
                    })
                    .context("query pending branch generation jobs")?;
                for row in rows {
                    let job = row.context("read branch generation job row")?;
                    tx.execute(
                        "UPDATE branch_artifact_versions
                         SET status = 'generating', updated_at = CURRENT_TIMESTAMP
                         WHERE id = ?1 AND status = 'pending'",
                        params![job.artifact_id],
                    )
                    .with_context(|| format!("mark branch artifact {} generating", job.artifact_id))?;
                    jobs.push(job);
                }
            }
            tx.commit()
                .context("commit branch generation claim transaction")?;
            Ok(jobs)
        })
    }

    pub fn mark_branch_generation_ready(
        &self,
        artifact_id: i64,
        tutorial_json: &str,
        html: &str,
        generated_head_sha: &str,
        unified_diff: &str,
    ) -> anyhow::Result<bool> {
        self.with_connection(|conn| {
            let tx = conn
                .unchecked_transaction()
                .context("start mark branch generation ready transaction")?;
            let (branch_id, current_status, source_head_sha, version): (i64, String, String, i64) =
                tx.query_row(
                    "SELECT branch_id, status, source_head_sha, version
                     FROM branch_artifact_versions
                     WHERE id = ?1",
                    params![artifact_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .with_context(|| format!("lookup branch artifact {}", artifact_id))?;
            let branch_head_sha: String = tx
                .query_row(
                    "SELECT head_sha
                     FROM branch_records
                     WHERE id = ?1",
                    params![branch_id],
                    |row| row.get(0),
                )
                .with_context(|| format!("lookup branch head for {}", branch_id))?;
            let newer_exists: bool = tx
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1
                        FROM branch_artifact_versions
                        WHERE branch_id = ?1 AND version > ?2 AND status != 'superseded'
                    )",
                    params![branch_id, version],
                    |row| row.get(0),
                )
                .with_context(|| format!("check newer branch artifacts for {}", branch_id))?;
            let activate = current_status != "superseded"
                && source_head_sha == branch_head_sha
                && !newer_exists;
            if activate {
                tx.execute(
                    "UPDATE branch_artifact_versions
                     SET is_current = 0
                     WHERE branch_id = ?1 AND is_current = 1",
                    params![branch_id],
                )
                .with_context(|| format!("clear current branch artifact for {}", branch_id))?;
            }
            tx.execute(
                "UPDATE branch_artifact_versions
                 SET status = 'ready',
                     tutorial_json = ?1,
                     html = ?2,
                     generated_head_sha = ?3,
                     unified_diff = ?4,
                     is_current = ?5,
                     ready_at = CURRENT_TIMESTAMP,
                     error_message = NULL,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?6",
                params![
                    tutorial_json,
                    html,
                    generated_head_sha,
                    unified_diff,
                    if activate { 1 } else { 0 },
                    artifact_id,
                ],
            )
            .with_context(|| format!("mark branch artifact {} ready", artifact_id))?;
            tx.commit()
                .context("commit mark branch generation ready transaction")?;
            Ok(activate)
        })
    }

    pub fn mark_branch_generation_failed(
        &self,
        artifact_id: i64,
        error_message: &str,
        retryable: bool,
        retry_backoff_secs: u64,
    ) -> anyhow::Result<()> {
        self.with_connection(|conn| {
            let next_retry = if retryable {
                Some(format!("+{} seconds", retry_backoff_secs))
            } else {
                None
            };
            let status = if retryable { "pending" } else { "failed" };
            conn.execute(
                "UPDATE branch_artifact_versions
                 SET status = ?1,
                     error_message = ?2,
                     retry_count = retry_count + 1,
                     next_retry_at = CASE
                        WHEN ?3 IS NULL THEN NULL
                        ELSE datetime('now', ?3)
                     END,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?4",
                params![status, error_message, next_retry, artifact_id],
            )
            .with_context(|| format!("mark branch artifact {} failed", artifact_id))?;
            Ok(())
        })
    }

    pub fn claim_pending_ci_runs(&self, limit: usize) -> anyhow::Result<Vec<PendingCiRun>> {
        self.with_connection(|conn| {
            let tx = conn
                .unchecked_transaction()
                .context("start ci claim transaction")?;
            let mut runs = Vec::new();
            {
                let mut stmt = tx
                    .prepare(
                        "SELECT id, branch_id, source_head_sha, entrypoint, command_json
                         FROM branch_ci_runs
                         WHERE status = 'queued'
                         ORDER BY id ASC
                         LIMIT ?1",
                    )
                    .context("prepare ci claim query")?;
                let rows = stmt
                    .query_map(params![limit as i64], |row| {
                        let command_json: String = row.get(4)?;
                        let command: Vec<String> =
                            serde_json::from_str(&command_json).unwrap_or_default();
                        Ok(PendingCiRun {
                            run_id: row.get(0)?,
                            branch_id: row.get(1)?,
                            source_head_sha: row.get(2)?,
                            entrypoint: row.get(3)?,
                            command,
                        })
                    })
                    .context("query queued ci runs")?;
                for row in rows {
                    let run = row.context("read queued ci run row")?;
                    tx.execute(
                        "UPDATE branch_ci_runs
                         SET status = 'running', started_at = CURRENT_TIMESTAMP
                         WHERE id = ?1 AND status = 'queued'",
                        params![run.run_id],
                    )
                    .with_context(|| format!("mark ci run {} running", run.run_id))?;
                    runs.push(run);
                }
            }
            tx.commit().context("commit ci claim transaction")?;
            Ok(runs)
        })
    }

    pub fn finish_ci_run(&self, run_id: i64, status: &str, log_text: &str) -> anyhow::Result<()> {
        self.with_connection(|conn| {
            conn.execute(
                "UPDATE branch_ci_runs
                 SET status = ?1, log_text = ?2, finished_at = CURRENT_TIMESTAMP
                 WHERE id = ?3",
                params![status, log_text, run_id],
            )
            .with_context(|| format!("finish ci run {}", run_id))?;
            Ok(())
        })
    }
}

fn ensure_repo_metadata(
    conn: &Connection,
    repo: &str,
    canonical_git_dir: &str,
    default_branch: &str,
    ci_entrypoint: &str,
) -> anyhow::Result<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM repos WHERE repo = ?1",
            params![repo],
            |row| row.get(0),
        )
        .optional()
        .context("lookup repo row")?;
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE repos
                 SET canonical_git_dir = ?1,
                     default_branch = ?2,
                     ci_entrypoint = ?3,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?4",
                params![canonical_git_dir, default_branch, ci_entrypoint, id],
            )
            .context("update repo metadata")?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO repos(repo, canonical_git_dir, default_branch, ci_entrypoint)
                 VALUES (?1, ?2, ?3, ?4)",
                params![repo, canonical_git_dir, default_branch, ci_entrypoint],
            )
            .context("insert repo metadata")?;
            Ok(conn.last_insert_rowid())
        }
    }
}

fn ensure_branch_artifact_for_head(
    conn: &Connection,
    branch_id: i64,
    head_sha: &str,
    merge_base_sha: &str,
) -> anyhow::Result<bool> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT source_head_sha
             FROM branch_artifact_versions
             WHERE branch_id = ?1 AND status != 'superseded'
             ORDER BY version DESC
             LIMIT 1",
            params![branch_id],
            |row| row.get(0),
        )
        .optional()
        .context("lookup existing branch artifact head")?;
    if existing.as_deref() == Some(head_sha) {
        return Ok(false);
    }

    conn.execute(
        "UPDATE branch_artifact_versions
         SET status = CASE WHEN is_current = 1 THEN status ELSE 'superseded' END,
             is_current = CASE WHEN is_current = 1 THEN is_current ELSE 0 END,
             updated_at = CURRENT_TIMESTAMP
         WHERE branch_id = ?1 AND is_current = 0 AND status != 'superseded'",
        params![branch_id],
    )
    .with_context(|| format!("supersede stale branch artifacts for {}", branch_id))?;
    let next_version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) + 1
             FROM branch_artifact_versions
             WHERE branch_id = ?1",
            params![branch_id],
            |row| row.get(0),
        )
        .with_context(|| format!("next branch artifact version for {}", branch_id))?;
    conn.execute(
        "INSERT INTO branch_artifact_versions(
            branch_id,
            version,
            source_head_sha,
            merge_base_sha,
            status
         ) VALUES (?1, ?2, ?3, ?4, 'pending')",
        params![branch_id, next_version, head_sha, merge_base_sha],
    )
    .with_context(|| format!("insert pending branch artifact for {}", branch_id))?;
    Ok(true)
}

fn ensure_ci_run_for_head(
    conn: &Connection,
    branch_id: i64,
    head_sha: &str,
    ci_entrypoint: &str,
    ci_command_json: &str,
) -> anyhow::Result<bool> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id
             FROM branch_ci_runs
             WHERE branch_id = ?1 AND source_head_sha = ?2
             ORDER BY id DESC
             LIMIT 1",
            params![branch_id, head_sha],
            |row| row.get(0),
        )
        .optional()
        .context("lookup existing ci run for head")?;
    if existing.is_some() {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO branch_ci_runs(branch_id, source_head_sha, entrypoint, command_json, status)
         VALUES (?1, ?2, ?3, ?4, 'queued')",
        params![branch_id, head_sha, ci_entrypoint, ci_command_json],
    )
    .with_context(|| format!("insert ci run for branch {}", branch_id))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::BranchUpsertInput;
    use crate::storage::Store;

    fn open_store() -> Store {
        let dir = tempfile::tempdir().expect("create temp dir").keep();
        let db_path = dir.join("pika-news.db");
        Store::open(&db_path).expect("open store")
    }

    fn upsert_input(branch_name: &str, head_sha: &str) -> BranchUpsertInput {
        BranchUpsertInput {
            repo: "sledtools/pika".to_string(),
            canonical_git_dir: "/tmp/pika.git".to_string(),
            default_branch: "master".to_string(),
            ci_entrypoint: "just pre-merge".to_string(),
            ci_command_json: serde_json::to_string(&vec!["just", "pre-merge"])
                .expect("serialize ci command"),
            branch_name: branch_name.to_string(),
            title: format!("{} title", branch_name),
            head_sha: head_sha.to_string(),
            merge_base_sha: "base123".to_string(),
            author_name: Some("alice".to_string()),
            author_email: Some("alice@example.com".to_string()),
            updated_at: "2026-03-16T12:00:00Z".to_string(),
        }
    }

    fn latest_branch_artifact_id(store: &Store, branch_id: i64) -> i64 {
        store
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT id
                     FROM branch_artifact_versions
                     WHERE branch_id = ?1
                     ORDER BY version DESC
                     LIMIT 1",
                    params![branch_id],
                    |row| row.get(0),
                )
                .map_err(Into::into)
            })
            .expect("lookup latest branch artifact")
    }

    #[test]
    fn branch_name_reuse_creates_new_numeric_record() {
        let store = open_store();
        let first = store
            .upsert_branch_record(&upsert_input("feature/reuse", "head111"))
            .expect("insert first branch lifecycle");
        store
            .mark_branch_closed(first.branch_id, "npub1trusted")
            .expect("close first branch lifecycle");

        let second = store
            .upsert_branch_record(&upsert_input("feature/reuse", "head222"))
            .expect("insert second branch lifecycle");

        assert_ne!(first.branch_id, second.branch_id);
        let items = store.list_branch_feed_items().expect("list branch feed");
        assert_eq!(items.len(), 2);
        assert!(items
            .iter()
            .any(|item| item.branch_id == first.branch_id && item.state == "closed"));
        assert!(items
            .iter()
            .any(|item| item.branch_id == second.branch_id && item.state == "open"));
    }

    #[test]
    fn head_change_queues_new_ci_run() {
        let store = open_store();
        let first = store
            .upsert_branch_record(&upsert_input("feature/ci", "head111"))
            .expect("insert branch");
        assert!(first.queued_ci);
        let second = store
            .upsert_branch_record(&upsert_input("feature/ci", "head222"))
            .expect("update branch with new head");
        assert!(second.head_changed);
        assert!(second.queued_ci);
        let runs = store
            .list_branch_ci_runs(second.branch_id, 10)
            .expect("list ci runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].source_head_sha, "head222");
        assert_eq!(runs[1].source_head_sha, "head111");
    }

    #[test]
    fn merged_branch_detail_remains_readable_after_deletion() {
        let store = open_store();
        let branch = store
            .upsert_branch_record(&upsert_input("feature/history", "head333"))
            .expect("insert branch");
        let artifact_id = latest_branch_artifact_id(&store, branch.branch_id);
        store
            .mark_branch_generation_ready(
                artifact_id,
                r#"{"executive_summary":"ok","media_links":[],"steps":[{"title":"Step","intent":"Intent","affected_files":["src/main.rs"],"evidence_snippets":["@@ -1 +1 @@"],"body_markdown":"body"}]}"#,
                "<p>ok</p>",
                "head333",
                "@@ -1 +1 @@",
            )
            .expect("mark branch artifact ready");
        store
            .finish_ci_run(1, "success", "ci ok")
            .expect("persist ci result");
        store
            .mark_branch_merged(branch.branch_id, "npub1trusted", "merge999")
            .expect("mark merged");

        let detail = store
            .get_branch_detail(branch.branch_id)
            .expect("load branch detail")
            .expect("detail exists");
        assert_eq!(detail.branch_state, "merged");
        assert_eq!(detail.merge_commit_sha.as_deref(), Some("merge999"));
        assert!(detail.tutorial_json.is_some());
        assert_eq!(detail.ci_status, "success");
        assert_eq!(detail.unified_diff.as_deref(), Some("@@ -1 +1 @@"));
    }
}
