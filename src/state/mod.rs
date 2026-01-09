pub mod checkpoint;
pub mod cleanup;
pub mod metadata;

pub use checkpoint::{Checkpoint, CheckpointManager, FailedRow, SessionStatus};
pub use cleanup::{CleanupManager, CleanupReport};
pub use metadata::{FileMetadata, MetadataStore, SessionMetadata};

use crate::utils::TranslationSessionConfig;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub config: TranslationSessionConfig,
    pub sessions: Arc<RwLock<HashMap<String, SessionState>>>,
}

#[derive(Debug, Clone)]
pub struct SessionState {
    pub session_id: String,
    pub status: SessionStatus,
    pub progress: f32,
    pub rows_processed: usize,
    pub rows_total: usize,
    pub chunks_completed: usize,
    pub chunks_total: usize,
    pub failed_rows_count: usize,
    pub start_time: u64,
    pub estimated_time_remaining: Option<u64>,
}

impl SessionState {
    pub fn new(session_id: String, rows_total: usize, chunks_total: usize) -> Self {
        Self {
            session_id,
            status: SessionStatus::Running,
            progress: 0.0,
            rows_processed: 0,
            rows_total,
            chunks_completed: 0,
            chunks_total,
            failed_rows_count: 0,
            start_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            estimated_time_remaining: None,
        }
    }

    pub fn update_progress(&mut self, rows_processed: usize, chunks_completed: usize) {
        self.rows_processed = rows_processed;
        self.chunks_completed = chunks_completed;
        self.progress = if self.rows_total > 0 {
            rows_processed as f32 / self.rows_total as f32
        } else {
            0.0
        };

        if self.chunks_completed > 0 {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let elapsed = now.saturating_sub(self.start_time);
            let rate = self.chunks_completed as f64 / elapsed.max(1) as f64;
            let remaining_chunks = self.chunks_total.saturating_sub(self.chunks_completed);
            self.estimated_time_remaining = Some((remaining_chunks as f64 / rate.max(0.001)) as u64);
        }
    }
}
