use crate::utils::{CsvTranslatorError, Result, TranslationSessionConfig};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

const CHECKPOINTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("checkpoints");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub session_id: String,
    pub input_file: String,
    pub output_file: String,
    pub config: TranslationSessionConfig,
    pub chunks_completed: Vec<usize>,
    pub last_completed_chunk: Option<usize>,
    pub rows_processed: usize,
    pub total_rows: usize,
    pub total_chunks: usize,
    pub failed_rows: Vec<FailedRow>,
    pub created_at: u64,
    pub updated_at: u64,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Paused,
    Completed,
    Failed,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Running => write!(f, "running"),
            SessionStatus::Paused => write!(f, "paused"),
            SessionStatus::Completed => write!(f, "completed"),
            SessionStatus::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedRow {
    pub row_index: usize,
    pub chunk_index: usize,
    pub error: String,
    pub original_content: Vec<String>,
    pub timestamp: u64,
}

pub struct CheckpointManager {
    db: Arc<Database>,
}

impl CheckpointManager {
    pub fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let db = Database::create(db_path)
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        let write_txn = db
            .begin_write()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
        {
            let _ = write_txn
                .open_table(CHECKPOINTS_TABLE)
                .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
        }
        write_txn
            .commit()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        Ok(Self { db: Arc::new(db) })
    }

    pub fn create_checkpoint(
        &self,
        session_id: &str,
        input_file: &str,
        output_file: &str,
        config: TranslationSessionConfig,
        total_rows: usize,
        total_chunks: usize,
    ) -> Result<Checkpoint> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let checkpoint = Checkpoint {
            session_id: session_id.to_string(),
            input_file: input_file.to_string(),
            output_file: output_file.to_string(),
            config,
            chunks_completed: Vec::new(),
            last_completed_chunk: None,
            rows_processed: 0,
            total_rows,
            total_chunks,
            failed_rows: Vec::new(),
            created_at: now,
            updated_at: now,
            status: SessionStatus::Running,
        };

        self.save_checkpoint(&checkpoint)?;
        Ok(checkpoint)
    }

    pub fn save_checkpoint(&self, checkpoint: &Checkpoint) -> Result<()> {
        let data = serde_json::to_vec(checkpoint)
            .map_err(|e| CsvTranslatorError::SerializationError(e.to_string()))?;

        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
        {
            let mut table = write_txn
                .open_table(CHECKPOINTS_TABLE)
                .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
            table
                .insert(checkpoint.session_id.as_str(), data.as_slice())
                .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
        }
        write_txn
            .commit()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    pub fn load_checkpoint(&self, session_id: &str) -> Result<Option<Checkpoint>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        let table = read_txn
            .open_table(CHECKPOINTS_TABLE)
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        match table.get(session_id) {
            Ok(Some(data)) => {
                let checkpoint: Checkpoint = serde_json::from_slice(data.value())
                    .map_err(|e| CsvTranslatorError::SerializationError(e.to_string()))?;
                Ok(Some(checkpoint))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(CsvTranslatorError::DatabaseError(e.to_string())),
        }
    }

    pub fn update_progress(
        &self,
        session_id: &str,
        chunk_index: usize,
        rows_in_chunk: usize,
    ) -> Result<()> {
        let mut checkpoint = self
            .load_checkpoint(session_id)?
            .ok_or_else(|| CsvTranslatorError::SessionNotFound(session_id.to_string()))?;

        checkpoint.chunks_completed.push(chunk_index);
        checkpoint.last_completed_chunk = Some(chunk_index);
        checkpoint.rows_processed += rows_in_chunk;
        checkpoint.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        if checkpoint.chunks_completed.len() >= checkpoint.total_chunks {
            checkpoint.status = SessionStatus::Completed;
        }

        self.save_checkpoint(&checkpoint)
    }

    pub fn add_failed_row(&self, session_id: &str, failed_row: FailedRow) -> Result<()> {
        let mut checkpoint = self
            .load_checkpoint(session_id)?
            .ok_or_else(|| CsvTranslatorError::SessionNotFound(session_id.to_string()))?;

        checkpoint.failed_rows.push(failed_row);
        checkpoint.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.save_checkpoint(&checkpoint)
    }

    pub fn set_status(&self, session_id: &str, status: SessionStatus) -> Result<()> {
        let mut checkpoint = self
            .load_checkpoint(session_id)?
            .ok_or_else(|| CsvTranslatorError::SessionNotFound(session_id.to_string()))?;

        checkpoint.status = status;
        checkpoint.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.save_checkpoint(&checkpoint)
    }

    pub fn delete_checkpoint(&self, session_id: &str) -> Result<()> {
        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
        {
            let mut table = write_txn
                .open_table(CHECKPOINTS_TABLE)
                .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
            table
                .remove(session_id)
                .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
        }
        write_txn
            .commit()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<String>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        let table = read_txn
            .open_table(CHECKPOINTS_TABLE)
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        let mut sessions = Vec::new();
        let iter = table
            .iter()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        for result in iter {
            let (key, _) = result.map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
            sessions.push(key.value().to_string());
        }

        Ok(sessions)
    }
}
