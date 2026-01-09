use crate::state::checkpoint::{CheckpointManager, SessionStatus};
use crate::utils::Result;
use std::path::Path;
use std::time::Duration;
use tokio::time::interval;
use tracing::{info, warn};

pub struct CleanupManager {
    checkpoint_manager: CheckpointManager,
    session_ttl_seconds: u64,
    max_temp_files_bytes: u64,
    temp_dir: String,
}

impl CleanupManager {
    pub fn new(
        checkpoint_manager: CheckpointManager,
        session_ttl_days: u64,
        max_temp_files_mb: u64,
        temp_dir: impl Into<String>,
    ) -> Self {
        Self {
            checkpoint_manager,
            session_ttl_seconds: session_ttl_days * 24 * 60 * 60,
            max_temp_files_bytes: max_temp_files_mb * 1024 * 1024,
            temp_dir: temp_dir.into(),
        }
    }

    pub async fn run_cleanup(&self) -> Result<CleanupReport> {
        let mut report = CleanupReport::default();

        report.sessions_deleted = self.cleanup_expired_sessions()?;

        report.temp_files_deleted = self.cleanup_temp_files()?;

        info!(
            sessions_deleted = report.sessions_deleted,
            temp_files_deleted = report.temp_files_deleted,
            bytes_freed = report.bytes_freed,
            "Cleanup completed"
        );

        Ok(report)
    }

    fn cleanup_expired_sessions(&self) -> Result<usize> {
        let sessions = self.checkpoint_manager.list_sessions()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut deleted = 0;

        for session_id in sessions {
            if let Ok(Some(checkpoint)) = self.checkpoint_manager.load_checkpoint(&session_id) {
                let age = now.saturating_sub(checkpoint.updated_at);

                let should_delete = match checkpoint.status {
                    SessionStatus::Completed | SessionStatus::Failed => {
                        age > self.session_ttl_seconds
                    }
                    SessionStatus::Paused => age > self.session_ttl_seconds * 2,
                    SessionStatus::Running => false,
                };

                if should_delete {
                    if let Err(e) = self.checkpoint_manager.delete_checkpoint(&session_id) {
                        warn!(session_id = %session_id, error = %e, "Failed to delete expired session");
                    } else {
                        deleted += 1;
                        info!(session_id = %session_id, "Deleted expired session");
                    }
                }
            }
        }

        Ok(deleted)
    }

    fn cleanup_temp_files(&self) -> Result<usize> {
        let temp_path = Path::new(&self.temp_dir);
        if !temp_path.exists() {
            return Ok(0);
        }

        let mut files_with_info: Vec<(std::path::PathBuf, u64, u64)> = Vec::new();
        let mut total_size = 0u64;

        if let Ok(entries) = std::fs::read_dir(temp_path) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        let size = metadata.len();
                        let modified = metadata
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);

                        files_with_info.push((entry.path(), size, modified));
                        total_size += size;
                    }
                }
            }
        }

        if total_size <= self.max_temp_files_bytes {
            return Ok(0);
        }

        files_with_info.sort_by_key(|f| f.2);

        let mut deleted = 0;
        let target_size = self.max_temp_files_bytes * 80 / 100;

        for (path, size, _) in files_with_info {
            if total_size <= target_size {
                break;
            }

            if let Err(e) = std::fs::remove_file(&path) {
                warn!(path = %path.display(), error = %e, "Failed to delete temp file");
            } else {
                total_size -= size;
                deleted += 1;
            }
        }

        Ok(deleted)
    }

    pub fn start_background_cleanup(self, cleanup_interval_hours: u64) {
        let interval_duration = Duration::from_secs(cleanup_interval_hours * 60 * 60);

        tokio::spawn(async move {
            let mut timer = interval(interval_duration);

            loop {
                timer.tick().await;

                match self.run_cleanup().await {
                    Ok(report) => {
                        info!(?report, "Background cleanup completed");
                    }
                    Err(e) => {
                        warn!(error = %e, "Background cleanup failed");
                    }
                }
            }
        });
    }
}

#[derive(Debug, Default)]
pub struct CleanupReport {
    pub sessions_deleted: usize,
    pub temp_files_deleted: usize,
    pub bytes_freed: u64,
}
