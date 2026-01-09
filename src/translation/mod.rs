pub mod cache;
pub mod client;
pub mod context;
pub mod long_cell;

pub use cache::TerminologyCache;
pub use client::{TranslationClient, TranslationContext};
pub use context::{build_system_prompt, ContextManager};
pub use long_cell::{ContentChunk, LongCellHandler};

use crate::csv_processor::{
    extract_core_rows, CsvChunker, CsvStreamReader, CsvStreamWriter,
};
use crate::utils::{CsvTranslatorError, Result, TranslationSessionConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TranslationSession {
    pub session_id: String,
    pub status: String,
    pub progress: f32,
    pub rows_processed: usize,
    pub rows_total: usize,
    pub estimated_time_remaining: Option<u64>,
    pub failed_rows_count: usize,
}

pub async fn start_translation_session(
    input_file: &str,
    output_file: &str,
    resume_from_checkpoint: Option<String>,
    config: &TranslationSessionConfig,
) -> Result<TranslationSession> {
    if !crate::csv_processor::file_exists(input_file) {
        return Err(CsvTranslatorError::FileNotFound(input_file.to_string()));
    }

    let session_id = resume_from_checkpoint.unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut reader = CsvStreamReader::new(input_file);
    let headers = reader.read_headers().await?;
    let total_rows = reader.count_rows().await?;

    let chunker = CsvChunker::new(config.chunk_size, config.overlap_size);
    let chunk_ranges = chunker.calculate_chunks(total_rows);
    let total_chunks = chunk_ranges.len();

    let session = TranslationSession {
        session_id: session_id.clone(),
        status: "running".to_string(),
        progress: 0.0,
        rows_processed: 0,
        rows_total: total_rows,
        estimated_time_remaining: None,
        failed_rows_count: 0,
    };

    let config_clone = config.clone();
    let input_file_clone = input_file.to_string();
    let output_file_clone = output_file.to_string();
    let headers_vec: Vec<String> = headers.iter().map(|s| s.to_string()).collect();

    tokio::spawn(async move {
        if let Err(e) = run_translation_pipeline(
            &session_id,
            &input_file_clone,
            &output_file_clone,
            &config_clone,
            headers_vec,
            total_rows,
            total_chunks,
        )
        .await
        {
            tracing::error!(session_id = %session_id, error = %e, "Translation pipeline failed");
        }
    });

    Ok(session)
}

async fn run_translation_pipeline(
    session_id: &str,
    input_file: &str,
    output_file: &str,
    config: &TranslationSessionConfig,
    headers: Vec<String>,
    total_rows: usize,
    _total_chunks: usize,
) -> Result<()> {
    let client = TranslationClient::new(config.clone());
    let mut context_manager = ContextManager::new(config.clone());
    let chunker = CsvChunker::new(config.chunk_size, config.overlap_size);
    let chunk_ranges = chunker.calculate_chunks(total_rows);

    let mut writer = CsvStreamWriter::new(output_file, headers.clone());
    writer.initialize()?;

    let reader = CsvStreamReader::new(input_file);
    let record_iter = reader.iter_records()?;

    let mut all_records: Vec<Vec<String>> = Vec::new();
    for result in record_iter {
        let (_, record) = result?;
        all_records.push(record.iter().map(|s| s.to_string()).collect());
    }

    for range in &chunk_ranges {
        let rows: Vec<Vec<String>> = all_records[range.start_row..range.end_row].to_vec();

        let chunk = chunker.create_chunk(range, rows, headers.clone());

        let context = context_manager.build_context();

        match client.translate_chunk(&chunk, &context).await {
            Ok(translated_rows) => {
                let core_rows = extract_core_rows(&chunk, translated_rows.clone());

                context_manager.update_from_translation(&translated_rows, config.overlap_size);

                let new_terms = ContextManager::extract_terminology_from_translation(
                    &chunk.rows,
                    &translated_rows,
                    &config.columns_to_translate,
                    &headers,
                );
                context_manager.add_terminologies(new_terms);

                writer.write_rows(&core_rows)?;

                tracing::info!(
                    session_id = %session_id,
                    chunk_index = range.index,
                    rows_written = core_rows.len(),
                    "Chunk translated successfully"
                );
            }
            Err(e) => {
                tracing::error!(
                    session_id = %session_id,
                    chunk_index = range.index,
                    error = %e,
                    "Failed to translate chunk, writing original"
                );

                let _core_start = range.core_start - range.start_row;
                let _core_end = range.core_end - range.start_row;
                let core_rows: Vec<Vec<String>> = all_records[range.core_start..range.core_end].to_vec();
                writer.write_rows(&core_rows)?;
            }
        }

        writer.flush()?;
    }

    writer.finish()?;

    tracing::info!(
        session_id = %session_id,
        total_rows = total_rows,
        "Translation completed"
    );

    Ok(())
}
