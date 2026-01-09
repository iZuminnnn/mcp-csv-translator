use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub translation: TranslationDefaults,
    pub api: ApiConfig,
    pub checkpoint: CheckpointConfig,
    pub cleanup: CleanupConfig,
    pub pipeline: PipelineConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationDefaults {
    pub default_chunk_size: usize,
    pub default_overlap_size: usize,
    pub max_tokens_per_request: usize,
    pub max_concurrent_requests: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    pub endpoint: String,
    pub model: String,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointConfig {
    pub auto_save_interval_seconds: u64,
    pub auto_save_every_n_chunks: usize,
    pub db_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupConfig {
    pub session_ttl_days: u64,
    pub cleanup_interval_hours: u64,
    pub max_temp_files_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    pub chunk_buffer_size: usize,
    pub resequencer_buffer_size: usize,
    pub failed_rows_output: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                name: "csv-translator-mcp".to_string(),
                version: "1.0.0".to_string(),
            },
            translation: TranslationDefaults {
                default_chunk_size: 500,
                default_overlap_size: 50,
                max_tokens_per_request: 120000,
                max_concurrent_requests: 5,
            },
            api: ApiConfig {
                endpoint: "https://api.anthropic.com/v1/messages".to_string(),
                model: "claude-3-5-sonnet-20241022".to_string(),
                timeout_seconds: 120,
            },
            checkpoint: CheckpointConfig {
                auto_save_interval_seconds: 60,
                auto_save_every_n_chunks: 5,
                db_path: PathBuf::from("./data/checkpoints.redb"),
            },
            cleanup: CleanupConfig {
                session_ttl_days: 7,
                cleanup_interval_hours: 1,
                max_temp_files_mb: 500,
            },
            pipeline: PipelineConfig {
                chunk_buffer_size: 10,
                resequencer_buffer_size: 20,
                failed_rows_output: PathBuf::from("./data/failed_rows.json"),
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "json".to_string(),
            },
        }
    }
}

impl AppConfig {
    pub fn load_from_file(path: &str) -> crate::utils::errors::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::utils::errors::CsvTranslatorError::ConfigError(e.to_string()))?;
        toml::from_str(&content)
            .map_err(|e| crate::utils::errors::CsvTranslatorError::ConfigError(e.to_string()))
    }

    pub fn load_or_default(path: Option<&str>) -> Self {
        if let Some(p) = path {
            Self::load_from_file(p).unwrap_or_default()
        } else {
            Self::default()
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TranslationSessionConfig {
    pub source_lang: String,
    pub target_lang: String,
    pub columns_to_translate: Vec<String>,
    pub chunk_size: usize,
    pub overlap_size: usize,
    pub domain_context: Option<String>,
    pub column_descriptions: Option<HashMap<String, String>>,
    pub max_tokens_per_request: usize,
    pub api_endpoint: String,
    pub api_key: String,
    pub model: String,
}
