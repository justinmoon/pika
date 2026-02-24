use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use crate::control::{ControlEnvelope, ControlState};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io error: {0}")]
    Io(String),
    #[error("json error: {0}")]
    Json(String),
    #[error("invalid operation log at line {line}: {message}")]
    InvalidOpLog { line: usize, message: String },
}

pub struct ControlStore {
    base_dir: PathBuf,
    snapshot_path: PathBuf,
    log_path: PathBuf,
}

impl ControlStore {
    pub fn new(base_dir: PathBuf) -> Self {
        let snapshot_path = base_dir.join("control_snapshot.json");
        let log_path = base_dir.join("control_ops.jsonl");
        Self {
            base_dir,
            snapshot_path,
            log_path,
        }
    }

    pub fn load_state(&self) -> Result<ControlState, StoreError> {
        if !self.base_dir.exists() {
            fs::create_dir_all(&self.base_dir).map_err(|e| StoreError::Io(e.to_string()))?;
        }

        let mut state = if self.snapshot_path.is_file() {
            let bytes = fs::read(&self.snapshot_path).map_err(|e| StoreError::Io(e.to_string()))?;
            serde_json::from_slice::<ControlState>(&bytes)
                .map_err(|e| StoreError::Json(e.to_string()))?
        } else {
            ControlState::default()
        };

        let mut ops = self.load_log_entries()?;
        ops.sort_by(log_order);
        for (line_no, op) in ops {
            state.apply(op).map_err(|e| StoreError::InvalidOpLog {
                line: line_no,
                message: e.to_string(),
            })?;
        }

        Ok(state)
    }

    pub fn load_ops(&self) -> Result<Vec<ControlEnvelope>, StoreError> {
        let mut ops = self.load_log_entries()?;
        ops.sort_by(log_order);
        Ok(ops.into_iter().map(|(_, op)| op).collect())
    }

    pub fn append_op(&self, op: &ControlEnvelope) -> Result<(), StoreError> {
        if !self.base_dir.exists() {
            fs::create_dir_all(&self.base_dir).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .map_err(|e| StoreError::Io(e.to_string()))?;
        let line = serde_json::to_string(op).map_err(|e| StoreError::Json(e.to_string()))?;
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|e| StoreError::Io(e.to_string()))
    }

    pub fn write_snapshot(&self, state: &ControlState) -> Result<(), StoreError> {
        if !self.base_dir.exists() {
            fs::create_dir_all(&self.base_dir).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let bytes =
            serde_json::to_vec_pretty(state).map_err(|e| StoreError::Json(e.to_string()))?;
        let tmp = self.base_dir.join("control_snapshot.tmp.json");
        fs::write(&tmp, bytes).map_err(|e| StoreError::Io(e.to_string()))?;
        fs::rename(tmp, &self.snapshot_path).map_err(|e| StoreError::Io(e.to_string()))
    }

    fn load_log_entries(&self) -> Result<Vec<(usize, ControlEnvelope)>, StoreError> {
        if !self.base_dir.exists() {
            fs::create_dir_all(&self.base_dir).map_err(|e| StoreError::Io(e.to_string()))?;
        }

        if !self.log_path.is_file() {
            return Ok(vec![]);
        }

        let file = fs::File::open(&self.log_path).map_err(|e| StoreError::Io(e.to_string()))?;
        let mut ops = vec![];
        for (idx, line_res) in BufReader::new(file).lines().enumerate() {
            let line_no = idx + 1;
            let line = line_res.map_err(|e| StoreError::Io(e.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let op = serde_json::from_str::<ControlEnvelope>(&line).map_err(|e| {
                StoreError::InvalidOpLog {
                    line: line_no,
                    message: e.to_string(),
                }
            })?;
            ops.push((line_no, op));
        }

        Ok(ops)
    }
}

fn log_order(a: &(usize, ControlEnvelope), b: &(usize, ControlEnvelope)) -> std::cmp::Ordering {
    a.1.ts_ms
        .cmp(&b.1.ts_ms)
        .then_with(|| a.1.op_id.cmp(&b.1.op_id))
}
