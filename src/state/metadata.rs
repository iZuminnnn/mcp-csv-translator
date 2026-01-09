use crate::utils::{CsvTranslatorError, Result};
use redb::{Database, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

const METADATA_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub file_path: String,
    pub file_hash: String,
    pub total_rows: usize,
    pub total_columns: usize,
    pub column_names: Vec<String>,
    pub file_size_bytes: u64,
    pub last_analyzed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub session_id: String,
    pub input_file_metadata: FileMetadata,
    pub output_file: String,
    pub terminology: HashMap<String, String>,
    pub context_summary: Option<String>,
}

pub struct MetadataStore {
    db: Arc<Database>,
}

impl MetadataStore {
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
                .open_table(METADATA_TABLE)
                .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
        }
        write_txn
            .commit()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        Ok(Self { db: Arc::new(db) })
    }

    pub fn save_file_metadata(&self, key: &str, metadata: &FileMetadata) -> Result<()> {
        let data = serde_json::to_vec(metadata)
            .map_err(|e| CsvTranslatorError::SerializationError(e.to_string()))?;

        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
        {
            let mut table = write_txn
                .open_table(METADATA_TABLE)
                .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
            table
                .insert(key, data.as_slice())
                .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
        }
        write_txn
            .commit()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    pub fn get_file_metadata(&self, key: &str) -> Result<Option<FileMetadata>> {
        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        let table = read_txn
            .open_table(METADATA_TABLE)
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        match table.get(key) {
            Ok(Some(data)) => {
                let metadata: FileMetadata = serde_json::from_slice(data.value())
                    .map_err(|e| CsvTranslatorError::SerializationError(e.to_string()))?;
                Ok(Some(metadata))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(CsvTranslatorError::DatabaseError(e.to_string())),
        }
    }

    pub fn save_session_metadata(&self, metadata: &SessionMetadata) -> Result<()> {
        let key = format!("session:{}", metadata.session_id);
        let data = serde_json::to_vec(metadata)
            .map_err(|e| CsvTranslatorError::SerializationError(e.to_string()))?;

        let write_txn = self
            .db
            .begin_write()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
        {
            let mut table = write_txn
                .open_table(METADATA_TABLE)
                .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
            table
                .insert(key.as_str(), data.as_slice())
                .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;
        }
        write_txn
            .commit()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    pub fn get_session_metadata(&self, session_id: &str) -> Result<Option<SessionMetadata>> {
        let key = format!("session:{}", session_id);

        let read_txn = self
            .db
            .begin_read()
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        let table = read_txn
            .open_table(METADATA_TABLE)
            .map_err(|e| CsvTranslatorError::DatabaseError(e.to_string()))?;

        match table.get(key.as_str()) {
            Ok(Some(data)) => {
                let metadata: SessionMetadata = serde_json::from_slice(data.value())
                    .map_err(|e| CsvTranslatorError::SerializationError(e.to_string()))?;
                Ok(Some(metadata))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(CsvTranslatorError::DatabaseError(e.to_string())),
        }
    }

    pub fn update_terminology(
        &self,
        session_id: &str,
        new_terms: HashMap<String, String>,
    ) -> Result<()> {
        let mut metadata = self
            .get_session_metadata(session_id)?
            .ok_or_else(|| CsvTranslatorError::SessionNotFound(session_id.to_string()))?;

        metadata.terminology.extend(new_terms);
        self.save_session_metadata(&metadata)
    }
}
