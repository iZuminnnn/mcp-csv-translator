use crate::csv_processor::{
    analyze_csv, chunker::string_record_to_vec, CsvChunker, CsvStreamReader, CsvStreamWriter,
};
use crate::state::{AppState, SessionState, SessionStatus};
use crate::translation::TranslationSession;
use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    ErrorData as McpError,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "Parameters for analyzing a CSV file")]
pub struct AnalyzeCsvParams {
    #[schemars(description = "Path to the CSV file to analyze")]
    pub file_path: String,
    #[schemars(description = "Number of sample rows to return (default: 10)")]
    pub sample_rows: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "Translation configuration parameters")]
pub struct ConfigureTranslationParams {
    #[schemars(description = "Source language code (e.g., 'vi', 'en', 'ja')")]
    pub source_lang: String,
    #[schemars(description = "Target language code")]
    pub target_lang: String,
    #[schemars(description = "List of column names to translate")]
    pub columns_to_translate: Vec<String>,
    #[schemars(description = "Number of rows per chunk (default: 50 for AI self-translation)")]
    pub chunk_size: Option<usize>,
    #[schemars(description = "Number of overlap rows between chunks (default: 5)")]
    pub overlap_size: Option<usize>,
    #[schemars(description = "Domain context for translation (e.g., 'medical', 'legal')")]
    pub domain_context: Option<String>,
    #[schemars(description = "Description for each column to help translation")]
    pub column_descriptions: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "Parameters to initialize a translation session (AI self-translation mode)")]
pub struct InitTranslationParams {
    #[schemars(description = "Path to input CSV file")]
    pub input_file: String,
    #[schemars(description = "Path to output CSV file")]
    pub output_file: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "Parameters to get next chunk for translation")]
pub struct GetNextChunkParams {
    #[schemars(description = "Session ID")]
    pub session_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "Parameters to submit translated chunk")]
pub struct SubmitTranslationParams {
    #[schemars(description = "Session ID")]
    pub session_id: String,
    #[schemars(description = "Chunk index being submitted")]
    pub chunk_index: usize,
    #[schemars(description = "Translated rows as array of arrays (only core rows, excluding overlap)")]
    pub translated_rows: Vec<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "Parameters to get translation progress")]
pub struct GetProgressParams {
    #[schemars(description = "Session ID to get progress for")]
    pub session_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkData {
    pub chunk_index: usize,
    pub total_chunks: usize,
    pub headers: Vec<String>,
    pub columns_to_translate: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub core_start_index: usize,
    pub core_end_index: usize,
    pub core_row_count: usize,
    pub source_lang: String,
    pub target_lang: String,
    pub domain_context: Option<String>,
    pub column_descriptions: Option<HashMap<String, String>>,
    pub instructions: String,
}

#[derive(Debug)]
struct TranslationSessionData {
    input_file: String,
    output_file: String,
    headers: Vec<String>,
    all_rows: Vec<Vec<String>>,
    chunk_ranges: Vec<crate::csv_processor::ChunkRange>,
    current_chunk: usize,
    translated_chunks: HashMap<usize, Vec<Vec<String>>>,
    writer_initialized: bool,
}

#[derive(Clone)]
pub struct CsvTranslatorServer {
    state: Arc<RwLock<AppState>>,
    sessions_data: Arc<RwLock<HashMap<String, TranslationSessionData>>>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl CsvTranslatorServer {
    pub fn new(state: AppState) -> Self {
        Self {
            state: Arc::new(RwLock::new(state)),
            sessions_data: Arc::new(RwLock::new(HashMap::new())),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "analyze_csv",
        description = "Analyze a CSV file to get metadata without loading entire file into memory. Returns row count, column names, file size, estimated tokens, and sample data."
    )]
    async fn analyze_csv(
        &self,
        params: Parameters<AnalyzeCsvParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let sample_rows = params.sample_rows.unwrap_or(10);

        let metadata = analyze_csv(&params.file_path, sample_rows)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let json_result = serde_json::to_string_pretty(&metadata)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json_result)]))
    }

    #[tool(
        name = "configure_translation",
        description = "Configure translation parameters including source/target languages, columns to translate, chunk size, and domain context. Must be called before init_translation."
    )]
    async fn configure_translation(
        &self,
        params: Parameters<ConfigureTranslationParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let mut state = self.state.write().await;

        state.config.source_lang = params.source_lang;
        state.config.target_lang = params.target_lang;
        state.config.columns_to_translate = params.columns_to_translate;
        state.config.chunk_size = params.chunk_size.unwrap_or(50);
        state.config.overlap_size = params.overlap_size.unwrap_or(5);
        state.config.domain_context = params.domain_context;
        state.config.column_descriptions = params.column_descriptions;

        Ok(CallToolResult::success(vec![Content::text(
            "Translation configured successfully. Now call init_translation to start.".to_string(),
        )]))
    }

    #[tool(
        name = "init_translation",
        description = "Initialize a translation session. This prepares the file for chunk-by-chunk translation by AI. Returns session_id and first chunk info. Use get_next_chunk to get chunks for translation."
    )]
    async fn init_translation(
        &self,
        params: Parameters<InitTranslationParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let state = self.state.read().await;

        if state.config.source_lang.is_empty() || state.config.target_lang.is_empty() {
            return Err(McpError::invalid_params(
                "Translation not configured. Call configure_translation first.",
                None,
            ));
        }

        let mut reader = CsvStreamReader::new(&params.input_file);
        let headers = reader
            .read_headers()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let headers_vec: Vec<String> = headers.iter().map(|s| s.to_string()).collect();

        let total_rows = reader
            .count_rows()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let mut all_rows: Vec<Vec<String>> = Vec::new();
        let record_iter = reader
            .iter_records()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        for result in record_iter {
            let (_, record) = result.map_err(|e| McpError::internal_error(e.to_string(), None))?;
            all_rows.push(string_record_to_vec(&record));
        }

        let chunker = CsvChunker::new(state.config.chunk_size, state.config.overlap_size);
        let chunk_ranges = chunker.calculate_chunks(total_rows);
        let total_chunks = chunk_ranges.len();

        let session_id = Uuid::new_v4().to_string();

        let session_data = TranslationSessionData {
            input_file: params.input_file.clone(),
            output_file: params.output_file.clone(),
            headers: headers_vec.clone(),
            all_rows,
            chunk_ranges,
            current_chunk: 0,
            translated_chunks: HashMap::new(),
            writer_initialized: false,
        };

        self.sessions_data
            .write()
            .await
            .insert(session_id.clone(), session_data);

        let mut sessions = state.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            SessionState::new(session_id.clone(), total_rows, total_chunks),
        );

        let response = serde_json::json!({
            "session_id": session_id,
            "total_rows": total_rows,
            "total_chunks": total_chunks,
            "headers": headers_vec,
            "columns_to_translate": state.config.columns_to_translate,
            "message": "Session initialized. Call get_next_chunk to get the first chunk for translation."
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    #[tool(
        name = "get_next_chunk",
        description = "Get the next chunk of data that needs translation. Returns the rows to translate with context. Translate ONLY the core rows (between core_start_index and core_end_index), overlap rows are for context only."
    )]
    async fn get_next_chunk(
        &self,
        params: Parameters<GetNextChunkParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let state = self.state.read().await;

        let sessions_data = self.sessions_data.read().await;
        let session_data = sessions_data.get(&params.session_id).ok_or_else(|| {
            McpError::invalid_params(format!("Session not found: {}", params.session_id), None)
        })?;

        let current_chunk = session_data.current_chunk;
        if current_chunk >= session_data.chunk_ranges.len() {
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::json!({
                    "status": "completed",
                    "message": "All chunks have been processed. Translation complete!"
                })
                .to_string(),
            )]));
        }

        let range = &session_data.chunk_ranges[current_chunk];
        
        // Bounds check
        let start_row = range.start_row.min(session_data.all_rows.len());
        let end_row = range.end_row.min(session_data.all_rows.len());
        
        if start_row >= end_row {
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::json!({
                    "status": "error",
                    "message": format!("Invalid range: start={}, end={}, total_rows={}", start_row, end_row, session_data.all_rows.len())
                })
                .to_string(),
            )]));
        }
        
        let rows: Vec<Vec<String>> = session_data.all_rows[start_row..end_row].to_vec();

        let core_start_in_chunk = (range.core_start - range.start_row).min(rows.len());
        let core_end_in_chunk = (range.core_end - range.start_row).min(rows.len());

        let instructions = format!(
            r#"TRANSLATION TASK:
- Translate from {} to {}
- Domain: {}
- Columns to translate: {:?}
- Keep other columns unchanged

IMPORTANT: 
- This chunk has {} total rows (indices 0 to {})
- ONLY translate rows from index {} to {} (exclusive) - these are the CORE rows
- Rows before index {} and after index {} are OVERLAP for context only
- Return EXACTLY {} translated rows (the core rows only)
- Preserve CSV structure exactly"#,
            state.config.source_lang,
            state.config.target_lang,
            state.config.domain_context.as_deref().unwrap_or("general"),
            state.config.columns_to_translate,
            rows.len(),
            rows.len() - 1,
            core_start_in_chunk,
            core_end_in_chunk,
            core_start_in_chunk,
            core_end_in_chunk,
            range.core_end - range.core_start
        );

        let chunk_data = ChunkData {
            chunk_index: current_chunk,
            total_chunks: session_data.chunk_ranges.len(),
            headers: session_data.headers.clone(),
            columns_to_translate: state.config.columns_to_translate.clone(),
            rows,
            core_start_index: core_start_in_chunk,
            core_end_index: core_end_in_chunk,
            core_row_count: range.core_end - range.core_start,
            source_lang: state.config.source_lang.clone(),
            target_lang: state.config.target_lang.clone(),
            domain_context: state.config.domain_context.clone(),
            column_descriptions: state.config.column_descriptions.clone(),
            instructions,
        };

        let json_result = serde_json::to_string_pretty(&chunk_data)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json_result)]))
    }

    #[tool(
        name = "submit_translation",
        description = "Submit translated rows for a chunk. The translated_rows should contain ONLY the core rows (not overlap rows). After submitting, call get_next_chunk for the next chunk."
    )]
    async fn submit_translation(
        &self,
        params: Parameters<SubmitTranslationParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;

        let mut sessions_data = self.sessions_data.write().await;
        let session_data = sessions_data.get_mut(&params.session_id).ok_or_else(|| {
            McpError::invalid_params(format!("Session not found: {}", params.session_id), None)
        })?;

        let range = session_data
            .chunk_ranges
            .get(params.chunk_index)
            .ok_or_else(|| McpError::invalid_params("Invalid chunk index", None))?;

        let expected_rows = range.core_end - range.core_start;
        if params.translated_rows.len() != expected_rows {
            return Err(McpError::invalid_params(
                format!(
                    "Row count mismatch: expected {} core rows, got {}",
                    expected_rows,
                    params.translated_rows.len()
                ),
                None,
            ));
        }

        session_data
            .translated_chunks
            .insert(params.chunk_index, params.translated_rows.clone());

        if !session_data.writer_initialized {
            let mut writer =
                CsvStreamWriter::new(&session_data.output_file, session_data.headers.clone());
            writer
                .initialize()
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            writer
                .write_rows(&params.translated_rows)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            writer
                .finish()
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            session_data.writer_initialized = true;
        } else {
            let file = std::fs::OpenOptions::new()
                .append(true)
                .open(&session_data.output_file)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            let mut writer = csv::Writer::from_writer(file);
            for row in &params.translated_rows {
                writer
                    .write_record(row)
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            }
            writer
                .flush()
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        }

        session_data.current_chunk += 1;

        let total_chunks = session_data.chunk_ranges.len();
        let chunks_done = session_data.current_chunk;
        let is_complete = chunks_done >= total_chunks;

        let state = self.state.read().await;
        let mut sessions = state.sessions.write().await;
        if let Some(session_state) = sessions.get_mut(&params.session_id) {
            let rows_done: usize = session_data
                .chunk_ranges
                .iter()
                .take(chunks_done)
                .map(|r| r.core_end - r.core_start)
                .sum();
            session_state.update_progress(rows_done, chunks_done);
            if is_complete {
                session_state.status = SessionStatus::Completed;
            }
        }

        let response = serde_json::json!({
            "status": if is_complete { "completed" } else { "in_progress" },
            "chunks_completed": chunks_done,
            "total_chunks": total_chunks,
            "progress_percent": (chunks_done as f64 / total_chunks as f64 * 100.0).round(),
            "message": if is_complete {
                format!("Translation complete! Output saved to: {}", session_data.output_file)
            } else {
                "Chunk saved. Call get_next_chunk for the next chunk.".to_string()
            }
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    #[tool(
        name = "get_translation_progress",
        description = "Get the current progress of a translation session."
    )]
    async fn get_translation_progress(
        &self,
        params: Parameters<GetProgressParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let state = self.state.read().await;

        let sessions = state.sessions.read().await;

        if let Some(session_state) = sessions.get(&params.session_id) {
            let response = TranslationSession {
                session_id: session_state.session_id.clone(),
                status: session_state.status.to_string(),
                progress: session_state.progress,
                rows_processed: session_state.rows_processed,
                rows_total: session_state.rows_total,
                estimated_time_remaining: session_state.estimated_time_remaining,
                failed_rows_count: session_state.failed_rows_count,
            };

            let json_result = serde_json::to_string_pretty(&response)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;

            Ok(CallToolResult::success(vec![Content::text(json_result)]))
        } else {
            Err(McpError::invalid_params(
                format!("Session not found: {}", params.session_id),
                None,
            ))
        }
    }

    pub fn router(&self) -> &ToolRouter<Self> {
        &self.tool_router
    }
}

#[tool_handler]
impl rmcp::handler::server::ServerHandler for CsvTranslatorServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                r#"CSV Translator MCP Server - Local Mode (AI Self-Translation)

Workflow:
1. analyze_csv - Analyze the CSV file (use absolute path)
2. configure_translation - Set languages, columns to translate, domain
3. init_translation - Initialize session, get session_id
4. Loop:
   - get_next_chunk - Get chunk with rows to translate
   - Translate the CORE rows (not overlap)
   - submit_translation - Submit translated rows
5. Repeat until complete

The AI translates chunks directly. All file paths are local paths."#
                    .to_string(),
            ),
        }
    }
}


