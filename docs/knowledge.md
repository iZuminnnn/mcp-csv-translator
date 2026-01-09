# MCP Server cho Dịch CSV Lớn - Technical Specification

## 1. Tổng quan dự án

### Mục tiêu
Xây dựng MCP (Model Context Protocol) server bằng Rust để dịch file CSV lớn (>1GB) một cách hiệu quả, tránh phình context window và mất ngữ cảnh.

### Nguyên tắc thiết kế
- **Memory-efficient**: Xử lý file lớn với RAM < 100MB
- **Context-aware**: Giữ ngữ cảnh khi dịch, không bị mất ý nghĩa
- **Performance-first**: Tận dụng Rust async/await và streaming I/O
- **Type-safe**: Zero runtime panic với Rust type system

---

## 2. Tech Stack

### Core Dependencies

```toml
[package]
name = "csv-translator-mcp"
version = "0.1.0"
edition = "2021"

[dependencies]
# MCP Framework (official Rust SDK)
# Features: server, transport-io (stdio), macros (tool/tool_router/tool_handler)
rmcp = { git = "https://github.com/modelcontextprotocol/rust-sdk", features = [
    "server",
    "transport-io", 
    "macros"
] }

# Async Runtime
tokio = { version = "1.35", features = ["full", "tracing"] }
async-trait = "0.1"

# CSV Processing
csv = "1.3"
tokio-util = { version = "0.7", features = ["codec", "io"] }
bytes = "1.5"

# HTTP Client & API
reqwest = { version = "0.11", features = ["json", "stream"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# State Management (redb thay thế sled - hiện đại hơn, được maintain tốt hơn)
redb = "2.1"

# Error Handling
thiserror = "1.0"
anyhow = "1.0"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# Token Counting (BẮT BUỘC - ước lượng chính xác hơn char length, đặc biệt với tiếng Việt)
tiktoken-rs = "0.5"

# Schema (tự động generate JSON Schema cho MCP tool parameters)
schemars = { version = "0.8", features = ["derive"] }

# UUID for session IDs
uuid = { version = "1.6", features = ["v4"] }
```

---

## 3. Kiến trúc hệ thống

### 3.1 Module Structure

```
src/
├── main.rs                 # Entry point, khởi tạo MCP server
├── lib.rs                  # Export các module
├── server/
│   ├── mod.rs              # MCP server setup
│   └── tools.rs            # Định nghĩa MCP tools
├── csv_processor/
│   ├── mod.rs
│   ├── reader.rs           # Streaming CSV reader
│   ├── chunker.rs          # Chunking logic với overlap
│   ├── writer.rs           # Streaming CSV writer
│   ├── analyzer.rs         # Metadata analysis
│   └── resequencer.rs      # Sắp xếp lại thứ tự chunks (xử lý concurrent out-of-order)
├── translation/
│   ├── mod.rs
│   ├── client.rs           # HTTP client cho translation API
│   ├── context.rs          # Context management & injection
│   ├── cache.rs            # Terminology cache
│   └── long_cell.rs        # Xử lý ô có nội dung dài
├── state/
│   ├── mod.rs
│   ├── checkpoint.rs       # Progress tracking & resume
│   ├── metadata.rs         # File metadata storage
│   └── cleanup.rs          # Session cleanup & TTL management
└── utils/
    ├── mod.rs
    ├── errors.rs           # Custom error types
    └── config.rs           # Configuration management
```

### 3.2 Data Flow

```
CSV Input → Stream Reader → Chunker (overlap) → Context Injector 
    ↓
Translation API Client ← Token Budget Manager (tiktoken-rs)
    ↓
Resequencer (đảm bảo thứ tự khi concurrent) → Discard Edges (bỏ overlap) → Stream Writer → CSV Output
    ↓
Checkpoint Manager (save progress)
```

**Lưu ý quan trọng về Resequencer**:
- Khi xử lý concurrent (5-10 chunks đồng thời), Chunk 2 có thể hoàn thành trước Chunk 1
- Resequencer giữ buffer chờ đến khi các chunks trước hoàn thành
- Giới hạn buffer size để tránh OOM khi chunk bị treo

**Chiến lược Discard Edges (thay vì Deduplicate)**:
- Chunk 1: Dịch rows 0-550 (overlap 50 cuối) → Chỉ lấy kết quả rows 0-500
- Chunk 2: Dịch rows 500-1050 (overlap 50 đầu + 50 cuối) → Chỉ lấy kết quả rows 550-1000
- Phần overlap chỉ dùng để AI hiểu ngữ cảnh, kết quả dịch bị bỏ đi
- Tránh vấn đề AI output không deterministic khi so sánh chuỗi

---

## 4. Core Components Specification

### 4.1 MCP Tools Definition

Định nghĩa 5 MCP tools chính:

#### Tool 1: `analyze_csv`
**Mô tả**: Phân tích metadata của file CSV mà không load toàn bộ vào RAM

**Input Schema**:
```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct AnalyzeCsvInput {
    file_path: String,
    sample_rows: Option<usize>, // Default: 10
}
```

**Output Schema**:
```rust
struct CsvMetadata {
    total_rows: usize,
    total_columns: usize,
    column_names: Vec<String>,
    file_size_bytes: u64,
    estimated_tokens: usize,
    sample_data: Vec<HashMap<String, String>>,
}
```

**Logic**:
- Đọc header và đếm số dòng bằng streaming
- Lấy mẫu N dòng đầu
- Estimate token count cho mỗi cột (average length × rows)
- Không load toàn bộ file vào memory

---

#### Tool 2: `configure_translation`
**Mô tả**: Cấu hình tham số dịch và context

**Input Schema**:
```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct TranslationConfig {
    source_lang: String,        // "vi", "en", "ja"...
    target_lang: String,
    columns_to_translate: Vec<String>, // Chỉ dịch các cột này
    chunk_size: usize,          // Default: 500 rows
    overlap_size: usize,        // Default: 50 rows (10%)
    domain_context: Option<String>, // "medical", "legal", "technical"
    column_descriptions: Option<HashMap<String, String>>,
    max_tokens_per_request: usize, // Default: 120000 (dành cho 128k context)
    api_endpoint: String,
    api_key: String,
}
```

**Output**: Configuration stored in state manager

---

#### Tool 3: `start_translation`
**Mô tả**: Bắt đầu quá trình dịch với streaming & checkpointing

**Input Schema**:
```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct StartTranslationInput {
    input_file: String,
    output_file: String,
    resume_from_checkpoint: Option<String>, // Resume nếu có checkpoint ID
}
```

**Output Schema**:
```rust
struct TranslationSession {
    session_id: String,
    status: String, // "running", "paused", "completed", "failed"
    progress: f32,  // 0.0 - 1.0
    rows_processed: usize,
    rows_total: usize,
    estimated_time_remaining: Option<u64>, // seconds
}
```

**Logic Flow**:
1. Load configuration từ state
2. Khởi tạo checkpoint manager
3. Stream read CSV → Chunker
4. Cho mỗi chunk:
   - Inject context (file summary, previous terms, column descriptions)
   - Calculate token count, ensure < max_tokens_per_request
   - Call translation API async
   - Merge results (deduplicate overlap zone)
   - Stream write to output file
   - Update checkpoint
5. Return session status

**Async Processing**:
```rust
// Pseudo-code
async fn process_chunk(
    chunk: CsvChunk,
    context: Arc<TranslationContext>,
    api_client: Arc<TranslationClient>,
) -> Result<TranslatedChunk> {
    // 1. Inject context
    let request = build_translation_request(chunk, context)?;

    // 2. Token budget check
    let token_count = estimate_tokens(&request);
    if token_count > context.config.max_tokens_per_request {
        return Err(Error::TokenLimitExceeded);
    }

    // 3. Call API
    let response = api_client.translate(request).await?;

    // 4. Validate output
    validate_translation_quality(&chunk, &response)?;

    Ok(response)
}
```

---

#### Tool 4: `get_translation_progress`
**Mô tả**: Lấy trạng thái tiến độ dịch

**Input**: `session_id: String`

**Output**: `TranslationSession` (như trên)

---

#### Tool 5: `handle_long_cell`
**Mô tả**: Xử lý riêng các ô có nội dung quá dài (>2000 tokens)

**Input Schema**:
```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct LongCellInput {
    cell_value: String,
    column_name: String,
    row_context: Vec<String>, // Các dòng trước/sau để tham khảo
    window_size: Option<usize>, // Default: 1500 tokens
    overlap: Option<usize>,     // Default: 200 tokens
}
```

**Logic**:
- Chia cell thành sub-chunks với sliding window
- Dịch từng sub-chunk kèm context (column name + row metadata)
- Merge kết quả, deduplicate ở overlap zones
- Return translated cell value

---

### 4.2 CSV Chunker với Overlap

```rust
pub struct CsvChunker {
    chunk_size: usize,
    overlap_size: usize,
}

pub struct CsvChunk {
    pub rows: Vec<CsvRow>,
    pub chunk_index: usize,
    pub has_overlap_start: bool, // Có overlap với chunk trước không
    pub has_overlap_end: bool,   // Có overlap với chunk sau không
}

impl CsvChunker {
    pub async fn chunk_with_overlap(
        &self,
        reader: csv::AsyncReader<impl AsyncRead>,
    ) -> impl Stream<Item = Result<CsvChunk>> {
        // Implementation:
        // 1. Đọc chunk_size + overlap_size rows
        // 2. Mark overlap_size rows cuối là overlap zone
        // 3. Yield chunk
        // 4. Keep overlap rows trong buffer cho chunk tiếp theo
        // 5. Repeat
    }
}
```

**Ví dụ**:
- Chunk size = 500, overlap = 50
- Chunk 1: rows 1-550 (rows 501-550 là overlap)
- Chunk 2: rows 501-1050 (rows 501-550 trùng với chunk 1, rows 1001-1050 là overlap)
- Khi merge: Bỏ duplicate ở overlap zone

---

### 4.3 Context Injection Strategy

```rust
pub struct TranslationContext {
    pub file_summary: String,         // "Medical records CSV, 50k rows, 20 columns"
    pub domain: String,                // "medical"
    pub column_descriptions: HashMap<String, String>,
    pub terminology_cache: Arc<RwLock<TerminologyCache>>,
}

pub struct TerminologyCache {
    terms: HashMap<String, String>, // source_term -> translated_term
    embeddings: Option<Vec<(String, Vec<f32>)>>, // For semantic lookup
}

impl TranslationContext {
    pub fn build_prompt(&self, chunk: &CsvChunk) -> String {
        format!(
            "File context: {}
Domain: {}

Column descriptions:
{}

Previously translated terms:
{}

Translate the following {} rows:
{}",
            self.file_summary,
            self.domain,
            format_column_descriptions(&self.column_descriptions),
            format_terminology_sample(&self.terminology_cache, 20), // Top 20 terms
            chunk.rows.len(),
            format_chunk_for_translation(chunk)
        )
    }
}
```

**Token Budget**:
- File summary: ~100 tokens
- Domain: ~10 tokens
- Column descriptions: ~20 tokens/column
- Terminology sample: ~300 tokens (20 terms × 15 tokens)
- Actual data: variable
- **Total overhead: ~500-1000 tokens** → Vẫn còn 120k+ tokens cho data

---

### 4.4 Checkpoint System (với redb)

```rust
use redb::{Database, TableDefinition, ReadableTable};

const CHECKPOINTS: TableDefinition<&str, &[u8]> = TableDefinition::new("checkpoints");

pub struct CheckpointManager {
    db: Database,
}

#[derive(Serialize, Deserialize)]
pub struct Checkpoint {
    pub session_id: String,
    pub last_completed_chunk: usize,
    pub last_completed_row: usize,
    pub terminology_cache: HashMap<String, String>,
    pub timestamp: u64,
    pub created_at: u64,  // Dùng cho TTL cleanup
}

impl CheckpointManager {
    pub fn new(path: &str) -> Result<Self> {
        let db = Database::create(path)?;
        Ok(Self { db })
    }

    pub fn save_checkpoint(&self, checkpoint: &Checkpoint) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(CHECKPOINTS)?;
            let value = serde_json::to_vec(checkpoint)?;
            table.insert(checkpoint.session_id.as_str(), value.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn load_checkpoint(&self, session_id: &str) -> Result<Option<Checkpoint>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(CHECKPOINTS)?;
        
        if let Some(value) = table.get(session_id)? {
            let checkpoint = serde_json::from_slice(value.value())?;
            Ok(Some(checkpoint))
        } else {
            Ok(None)
        }
    }
}
```

**Auto-save frequency**: Sau mỗi 5 chunks hoặc mỗi 60 giây

---

### 4.5 Translation API Client

```rust
pub struct TranslationClient {
    http_client: reqwest::Client,
    api_endpoint: String,
    api_key: String,
    rate_limiter: Arc<Semaphore>, // Giới hạn concurrent requests
}

#[derive(Serialize)]
struct TranslationRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    max_tokens: usize,
}

impl TranslationClient {
    pub async fn translate(&self, request: TranslationRequest) -> Result<String> {
        // 1. Acquire rate limit permit
        let _permit = self.rate_limiter.acquire().await?;

        // 2. Send request
        let response = self.http_client
            .post(&self.api_endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await?;

        // 3. Parse response
        let result: ApiResponse = response.json().await?;

        // 4. Extract translated text
        Ok(result.choices[0].message.content.clone())
    }

    pub async fn translate_with_retry(
        &self,
        request: TranslationRequest,
        max_retries: usize,
    ) -> Result<String> {
        let mut retries = 0;
        loop {
            match self.translate(request.clone()).await {
                Ok(result) => return Ok(result),
                Err(e) if retries < max_retries => {
                    retries += 1;
                    tracing::warn!("Translation failed, retry {}/{}: {}", retries, max_retries, e);
                    tokio::time::sleep(Duration::from_secs(2u64.pow(retries as u32))).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}
```

---

### 4.6 Discard Edges Strategy (thay thế Deduplicate)

```rust
pub struct DiscardEdgesProcessor {
    overlap_size: usize,
}

impl DiscardEdgesProcessor {
    pub fn new(overlap_size: usize) -> Self {
        Self { overlap_size }
    }

    /// Trích xuất phần "core" của chunk (bỏ overlap edges)
    /// - Chunk đầu tiên: bỏ overlap cuối
    /// - Chunk giữa: bỏ overlap đầu VÀ cuối
    /// - Chunk cuối: bỏ overlap đầu
    pub fn extract_core_rows(
        &self,
        chunk: &TranslatedChunk,
        is_first: bool,
        is_last: bool,
    ) -> Vec<CsvRow> {
        let total = chunk.rows.len();
        
        let start = if is_first { 0 } else { self.overlap_size };
        let end = if is_last { total } else { total - self.overlap_size };
        
        chunk.rows[start..end].to_vec()
    }
}
```

**Ưu điểm so với Deduplicate**:
- Không phụ thuộc vào việc so sánh chuỗi (AI output không deterministic)
- Logic đơn giản, dễ debug
- Performance tốt hơn (không cần hash/compare)

---

### 4.7 Resequencer (Xử lý Concurrent Out-of-Order)

```rust
use std::collections::BTreeMap;
use tokio::sync::mpsc;

pub struct Resequencer {
    buffer: BTreeMap<usize, TranslatedChunk>,  // chunk_index -> chunk
    next_expected: usize,
    max_buffer_size: usize,  // Giới hạn để tránh OOM
}

impl Resequencer {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            buffer: BTreeMap::new(),
            next_expected: 0,
            max_buffer_size,
        }
    }

    /// Nhận chunk và trả về các chunks đã sẵn sàng để ghi (theo thứ tự)
    pub fn receive(&mut self, chunk: TranslatedChunk) -> Result<Vec<TranslatedChunk>, ResequencerError> {
        // Check buffer overflow (chunk bị treo quá lâu)
        if self.buffer.len() >= self.max_buffer_size {
            return Err(ResequencerError::BufferOverflow {
                waiting_for: self.next_expected,
                buffer_size: self.buffer.len(),
            });
        }

        self.buffer.insert(chunk.chunk_index, chunk);

        // Trả về tất cả chunks liên tiếp từ next_expected
        let mut ready = Vec::new();
        while let Some(chunk) = self.buffer.remove(&self.next_expected) {
            ready.push(chunk);
            self.next_expected += 1;
        }

        Ok(ready)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResequencerError {
    #[error("Buffer overflow: waiting for chunk {waiting_for}, buffer has {buffer_size} chunks")]
    BufferOverflow { waiting_for: usize, buffer_size: usize },
}
```

**Cấu hình khuyến nghị**:
- `max_buffer_size`: 10-20 chunks (với mỗi chunk ~2-3MB, tối đa ~60MB buffer)
- Nếu buffer overflow: giảm concurrency hoặc timeout chunk bị treo

---

### 4.8 Session Cleanup & TTL

```rust
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct SessionCleanup {
    db: Database,
    ttl: Duration,  // Thời gian sống của session (mặc định: 7 ngày)
}

const SESSIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("sessions");

impl SessionCleanup {
    pub fn new(db: Database, ttl: Duration) -> Self {
        Self { db, ttl }
    }

    /// Dọn dẹp các sessions hết hạn
    pub fn cleanup_expired(&self) -> Result<usize> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let ttl_secs = self.ttl.as_secs();
        let mut deleted = 0;

        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SESSIONS)?;
            let mut to_delete = Vec::new();

            // Tìm các sessions hết hạn
            for entry in table.iter()? {
                let (key, value) = entry?;
                let session: Session = serde_json::from_slice(value.value())?;
                
                if now - session.created_at > ttl_secs {
                    to_delete.push(key.value().to_string());
                }
            }

            // Xóa
            for key in to_delete {
                table.remove(key.as_str())?;
                deleted += 1;
            }
        }
        write_txn.commit()?;

        tracing::info!(deleted_sessions = deleted, "Cleanup completed");
        Ok(deleted)
    }

    /// Chạy cleanup định kỳ (mỗi 1 giờ)
    pub async fn run_periodic(self: Arc<Self>) {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if let Err(e) = self.cleanup_expired() {
                tracing::error!(error = %e, "Cleanup failed");
            }
        }
    }
}
```

---

### 4.9 Backpressure với Bounded Channels

```rust
use tokio::sync::mpsc;

pub struct TranslationPipeline {
    chunk_buffer_size: usize,  // Giới hạn số chunks trong buffer
}

impl TranslationPipeline {
    pub async fn run(&self, input_file: &str, output_file: &str) -> Result<()> {
        // Bounded channels để tạo backpressure
        // Nếu buffer đầy, reader sẽ bị block → tránh OOM
        let (chunk_tx, chunk_rx) = mpsc::channel::<CsvChunk>(self.chunk_buffer_size);
        let (translated_tx, translated_rx) = mpsc::channel::<TranslatedChunk>(self.chunk_buffer_size);

        // Spawn tasks
        let reader_handle = tokio::spawn(self.read_and_chunk(input_file, chunk_tx));
        let translator_handle = tokio::spawn(self.translate_chunks(chunk_rx, translated_tx));
        let writer_handle = tokio::spawn(self.resequence_and_write(translated_rx, output_file));

        // Wait for all
        let (r1, r2, r3) = tokio::try_join!(reader_handle, translator_handle, writer_handle)?;
        r1?; r2?; r3?;

        Ok(())
    }

    async fn read_and_chunk(
        &self,
        input_file: &str,
        tx: mpsc::Sender<CsvChunk>,
    ) -> Result<()> {
        let reader = CsvReader::open(input_file)?;
        let chunker = CsvChunker::new(500, 50);

        let mut stream = chunker.chunk_with_overlap(reader);
        while let Some(chunk) = stream.next().await {
            // Nếu channel đầy, send() sẽ block ở đây → backpressure
            tx.send(chunk?).await.map_err(|_| Error::ChannelClosed)?;
        }
        Ok(())
    }
}
```

**Cấu hình khuyến nghị**:
- `chunk_buffer_size`: 5-10 (với mỗi chunk ~500 rows, ~2MB text)
- Memory budget: ~20-50MB cho pipeline buffers

---

### 4.10 Partial Failure Handling

```rust
pub struct PartialFailureHandler;

#[derive(Debug)]
pub enum RowResult {
    Translated(CsvRow),
    Failed { original: CsvRow, error: String },
}

impl PartialFailureHandler {
    /// Xử lý chunk có một số rows bị lỗi
    /// Không fail cả chunk vì 1 row lỗi
    pub fn handle_chunk_with_failures(
        &self,
        original: &CsvChunk,
        translated: Result<TranslatedChunk, TranslationError>,
    ) -> (Vec<CsvRow>, Vec<FailedRow>) {
        match translated {
            Ok(trans) => {
                // Validate row count
                if original.rows.len() != trans.rows.len() {
                    tracing::warn!(
                        expected = original.rows.len(),
                        got = trans.rows.len(),
                        "Row count mismatch - AI hallucination detected"
                    );
                    // Fallback: giữ nguyên original và đánh dấu warning
                    return self.fallback_to_original(original);
                }
                
                (trans.rows, Vec::new())
            }
            Err(e) => {
                tracing::error!(error = %e, "Chunk translation failed, using original");
                self.fallback_to_original(original)
            }
        }
    }

    fn fallback_to_original(&self, original: &CsvChunk) -> (Vec<CsvRow>, Vec<FailedRow>) {
        let failed: Vec<FailedRow> = original.rows.iter().map(|row| FailedRow {
            row_id: row.id,
            original_content: row.clone(),
            error: "Translation failed".to_string(),
        }).collect();

        (original.rows.clone(), failed)
    }
}

#[derive(Debug, Serialize)]
pub struct FailedRow {
    pub row_id: usize,
    pub original_content: CsvRow,
    pub error: String,
}

// Output file cho manual review
pub async fn write_failed_rows(failed: &[FailedRow], path: &str) -> Result<()> {
    let json = serde_json::to_string_pretty(failed)?;
    tokio::fs::write(path, json).await?;
    Ok(())
}
```

**Ưu điểm**:
- Không mất toàn bộ progress vì 1 row lỗi
- Tạo file review cho manual fix
- Log đầy đủ để debug

---

## 5. Quality Assurance Features

### 5.1 Validation Layer

```rust
pub struct TranslationValidator;

impl TranslationValidator {
    pub fn validate_chunk(
        &self,
        original: &CsvChunk,
        translated: &TranslatedChunk,
    ) -> Result<ValidationReport> {
        let mut report = ValidationReport::default();

        // Check 1: Row count match
        if original.rows.len() != translated.rows.len() {
            report.errors.push("Row count mismatch".to_string());
        }

        // Check 2: Column count match
        for (orig_row, trans_row) in original.rows.iter().zip(&translated.rows) {
            if orig_row.len() != trans_row.len() {
                report.warnings.push(format!("Column count mismatch at row {}", orig_row.id));
            }
        }

        // Check 3: Length ratio (translated text shouldn't be 10x longer)
        for (orig_row, trans_row) in original.rows.iter().zip(&translated.rows) {
            let orig_len: usize = orig_row.values().map(|v| v.len()).sum();
            let trans_len: usize = trans_row.values().map(|v| v.len()).sum();

            if trans_len > orig_len * 10 {
                report.warnings.push(format!("Suspicious length ratio at row {}", orig_row.id));
            }
        }

        // Check 4: Empty cells check
        for (i, trans_row) in translated.rows.iter().enumerate() {
            if original.rows[i].values().any(|v| !v.is_empty()) 
                && trans_row.values().all(|v| v.is_empty()) {
                report.errors.push(format!("All cells empty after translation at row {}", i));
            }
        }

        Ok(report)
    }
}
```

### 5.2 Error Handling Strategies

```rust
#[derive(Debug, thiserror::Error)]
pub enum TranslationError {
    #[error("CSV parsing error: {0}")]
    CsvError(#[from] csv::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Token limit exceeded: {current} > {max}")]
    TokenLimitExceeded { current: usize, max: usize },

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Checkpoint error: {0}")]
    CheckpointError(String),

    #[error("Row count mismatch: expected {expected}, got {got}")]
    RowCountMismatch { expected: usize, got: usize },

    #[error("Resequencer buffer overflow: waiting for chunk {waiting_for}")]
    ResequencerOverflow { waiting_for: usize },

    #[error("Channel closed unexpectedly")]
    ChannelClosed,
}

// Retry strategy
pub enum RetryStrategy {
    ExpandContext,   // Retry với +2 rows context
    ReduceChunkSize, // Giảm chunk size xuống 50%
    SkipRow,         // Skip row lỗi, log để review sau
    LowerTemperature, // Retry với temperature thấp hơn (strict mode)
}
```

---

## 6. Configuration & Deployment

### 6.1 Configuration File

`config.toml`:
```toml
[server]
name = "csv-translator-mcp"
version = "1.0.0"

[translation]
default_chunk_size = 500
default_overlap_size = 50
max_tokens_per_request = 120000
max_concurrent_requests = 5

[api]
endpoint = "https://api.anthropic.com/v1/messages"
model = "claude-3-5-sonnet-20241022"
timeout_seconds = 120

[checkpoint]
auto_save_interval_seconds = 60
auto_save_every_n_chunks = 5
db_path = "./data/checkpoints.redb"

[cleanup]
session_ttl_days = 7           # Xóa sessions sau 7 ngày
cleanup_interval_hours = 1     # Chạy cleanup mỗi 1 giờ
max_temp_files_mb = 500        # Giới hạn dung lượng temp files

[pipeline]
chunk_buffer_size = 10         # Bounded channel size (backpressure)
resequencer_buffer_size = 20   # Max chunks chờ trong resequencer
failed_rows_output = "./data/failed_rows.json"

[logging]
level = "info"
format = "json"
```

### 6.2 MCP Server Setup (main.rs) - Cập nhật theo rmcp SDK mới nhất

```rust
use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError,
    ServiceExt,
    Json,
    handler::server::wrapper::Parameters,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

mod csv_processor;
mod translation;
mod state;
mod utils;

// ============ Input/Output Schemas ============

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "Parameters for analyzing a CSV file")]
pub struct AnalyzeCsvParams {
    #[schemars(description = "Path to the CSV file to analyze")]
    pub file_path: String,
    #[schemars(description = "Number of sample rows to return (default: 10)")]
    pub sample_rows: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CsvMetadata {
    pub total_rows: usize,
    pub total_columns: usize,
    pub column_names: Vec<String>,
    pub file_size_bytes: u64,
    pub estimated_tokens: usize,
    pub sample_data: Vec<serde_json::Value>,
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
    #[schemars(description = "Number of rows per chunk (default: 500)")]
    pub chunk_size: Option<usize>,
    #[schemars(description = "Number of overlap rows between chunks (default: 50)")]
    pub overlap_size: Option<usize>,
    #[schemars(description = "Domain context for translation (e.g., 'medical', 'legal')")]
    pub domain_context: Option<String>,
    #[schemars(description = "Description for each column to help translation")]
    pub column_descriptions: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(description = "Parameters to start translation")]
pub struct StartTranslationParams {
    #[schemars(description = "Path to input CSV file")]
    pub input_file: String,
    #[schemars(description = "Path to output CSV file")]
    pub output_file: String,
    #[schemars(description = "Resume from checkpoint ID if available")]
    pub resume_from_checkpoint: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TranslationSession {
    pub session_id: String,
    pub status: String,
    pub progress: f32,
    pub rows_processed: usize,
    pub rows_total: usize,
    pub estimated_time_remaining: Option<u64>,
    pub failed_rows_count: usize,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetProgressParams {
    #[schemars(description = "Session ID to get progress for")]
    pub session_id: String,
}

// ============ Server State ============

#[derive(Clone)]
pub struct CsvTranslatorServer {
    state: Arc<RwLock<state::AppState>>,
    tool_router: ToolRouter<Self>,
}

// ============ Tool Implementations ============

#[tool_router]
impl CsvTranslatorServer {
    pub fn new(state: state::AppState) -> Self {
        Self {
            state: Arc::new(RwLock::new(state)),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "analyze_csv",
        description = "Analyze a CSV file to get metadata without loading entire file into memory. Returns row count, column names, file size, estimated tokens, and sample data."
    )]
    async fn analyze_csv(
        &self,
        Parameters(params): Parameters<AnalyzeCsvParams>,
    ) -> Result<Json<CsvMetadata>, McpError> {
        let sample_rows = params.sample_rows.unwrap_or(10);
        
        let metadata = csv_processor::analyzer::analyze_csv(
            &params.file_path,
            sample_rows,
        ).await.map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(Json(metadata))
    }

    #[tool(
        name = "configure_translation",
        description = "Configure translation parameters including source/target languages, columns to translate, chunk size, and domain context. Must be called before start_translation."
    )]
    async fn configure_translation(
        &self,
        Parameters(params): Parameters<ConfigureTranslationParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.write().await;
        
        state.config.source_lang = params.source_lang;
        state.config.target_lang = params.target_lang;
        state.config.columns_to_translate = params.columns_to_translate;
        state.config.chunk_size = params.chunk_size.unwrap_or(500);
        state.config.overlap_size = params.overlap_size.unwrap_or(50);
        state.config.domain_context = params.domain_context;
        state.config.column_descriptions = params.column_descriptions;

        Ok(CallToolResult::success(vec![Content::text(
            "Translation configured successfully".to_string(),
        )]))
    }

    #[tool(
        name = "start_translation",
        description = "Start translating a CSV file with the configured settings. Returns a session ID to track progress. Use get_translation_progress to monitor status."
    )]
    async fn start_translation(
        &self,
        Parameters(params): Parameters<StartTranslationParams>,
    ) -> Result<Json<TranslationSession>, McpError> {
        let state = self.state.read().await;
        
        let session = translation::start_translation_session(
            &params.input_file,
            &params.output_file,
            params.resume_from_checkpoint,
            &state.config,
        ).await.map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(Json(session))
    }

    #[tool(
        name = "get_translation_progress",
        description = "Get the current progress of a translation session. Returns status, progress percentage, rows processed, and estimated time remaining."
    )]
    async fn get_translation_progress(
        &self,
        Parameters(params): Parameters<GetProgressParams>,
    ) -> Result<Json<TranslationSession>, McpError> {
        let state = self.state.read().await;
        
        let session = state.get_session(&params.session_id)
            .ok_or_else(|| McpError::invalid_params(
                format!("Session not found: {}", params.session_id),
                None,
            ))?;

        Ok(Json(session))
    }
}

// ============ Server Handler ============

#[tool_handler]
impl rmcp::ServerHandler for CsvTranslatorServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .build(),
            server_info: Implementation {
                name: "csv-translator-mcp".to_string(),
                version: "1.0.0".to_string(),
            },
            instructions: Some(
                "MCP server for translating large CSV files efficiently. \
                 Supports streaming, checkpointing, and context-aware translation.".into()
            ),
            ..Default::default()
        }
    }
}

// ============ Main Entry Point ============

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("csv_translator_mcp=info")
        .json()
        .init();

    // Load configuration
    let config = utils::config::load_config("config.toml")?;

    // Initialize shared state
    let app_state = state::AppState::new(config).await?;

    // Create and run MCP server with STDIO transport
    let server = CsvTranslatorServer::new(app_state);
    let service = server.serve(stdio()).await?;
    
    tracing::info!("CSV Translator MCP server started");
    
    // Wait for service to complete
    service.waiting().await?;

    Ok(())
}
```

**Giải thích cấu trúc rmcp:**
1. **`#[tool_router]`** - Macro đánh dấu impl block chứa các tools
2. **`#[tool(...)]`** - Macro đánh dấu function là MCP tool với name và description
3. **`Parameters<T>`** - Wrapper để parse input từ JSON với schema từ `JsonSchema` derive
4. **`Json<T>`** - Return type để serialize output thành JSON
5. **`#[tool_handler]`** - Macro implement `ServerHandler` trait với tool routing
6. **`serve(stdio())`** - Chạy server với STDIO transport (cho Claude Desktop)

### 6.3 Claude Desktop Integration

`claude_desktop_config.json`:
```json
{
  "mcpServers": {
    "csv-translator": {
      "command": "/path/to/csv-translator-mcp/target/release/csv-translator-mcp",
      "args": [],
      "env": {
        "RUST_LOG": "info",
        "CONFIG_PATH": "/path/to/config.toml"
      }
    }
  }
}
```

---

## 7. Testing Strategy

### 7.1 Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_chunker_with_overlap() {
        let chunker = CsvChunker::new(100, 10);
        let csv_data = generate_test_csv(500);

        let chunks: Vec<_> = chunker.chunk_with_overlap(csv_data).collect().await;

        assert_eq!(chunks.len(), 5); // 500 rows / 100 chunk_size = 5 chunks
        assert_eq!(chunks[0].rows.len(), 110); // 100 + 10 overlap
        assert_eq!(chunks[1].rows.len(), 110); // Same

        // Check overlap correctness
        assert_eq!(
            chunks[0].rows[100..110],
            chunks[1].rows[0..10]
        );
    }

    #[test]
    fn test_token_budget_calculation() {
        let config = TranslationConfig {
            max_tokens_per_request: 120000,
            ..Default::default()
        };

        let chunk = CsvChunk { /* ... */ };
        let context = TranslationContext { /* ... */ };

        let estimated = estimate_tokens_for_chunk(&chunk, &context);

        assert!(estimated < 120000, "Token budget exceeded");
    }

    #[tokio::test]
    async fn test_checkpoint_save_load() {
        let manager = CheckpointManager::new("test_db").await.unwrap();

        let checkpoint = Checkpoint {
            session_id: "test-123".to_string(),
            last_completed_chunk: 5,
            last_completed_row: 500,
            terminology_cache: HashMap::new(),
            timestamp: 12345,
        };

        manager.save_checkpoint(checkpoint.clone()).await.unwrap();

        let loaded = manager.load_checkpoint("test-123").unwrap();
        assert_eq!(loaded.unwrap().last_completed_chunk, 5);
    }
}
```

### 7.2 Integration Tests

```rust
#[tokio::test]
async fn test_end_to_end_translation() {
    // 1. Create test CSV with 1000 rows
    let test_csv = create_test_csv(1000);

    // 2. Start translation
    let session = start_translation(StartTranslationInput {
        input_file: test_csv.path().to_str().unwrap().to_string(),
        output_file: "/tmp/output.csv".to_string(),
        resume_from_checkpoint: None,
    }).await.unwrap();

    // 3. Wait for completion
    loop {
        let progress = get_translation_progress(session.session_id.clone()).await.unwrap();
        if progress.status == "completed" {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // 4. Validate output
    let output = std::fs::read_to_string("/tmp/output.csv").unwrap();
    assert!(output.len() > 0);

    let output_rows = count_csv_rows("/tmp/output.csv").unwrap();
    assert_eq!(output_rows, 1000);
}
```

---

## 8. Performance Targets

### 8.1 Benchmarks

- **Throughput**: > 1000 rows/second cho CSV đơn giản (50 chars/cell)
- **Memory**: < 100MB RAM cho file 1GB
- **Latency**: < 500ms overhead per chunk (excluding API call time)
- **Concurrency**: Xử lý 5-10 chunks đồng thời (tùy API rate limit)

### 8.2 Optimization Techniques

1. **Zero-copy parsing**: Dùng `bytes::Bytes` và `csv::ByteRecord`
2. **Async streaming**: Không chờ chunk hoàn thành mới xử lý chunk tiếp
3. **Connection pooling**: Reuse HTTP connections
4. **Batch API calls**: Nếu API hỗ trợ, gửi multiple chunks trong 1 request

---

## 9. Implementation Roadmap

### Phase 1: Core Infrastructure (Week 1)
- [ ] Setup project structure
- [ ] Implement CSV streaming reader/writer
- [ ] Implement chunker with overlap
- [ ] Basic MCP server skeleton

### Phase 2: Translation Logic (Week 2)
- [ ] Translation API client with retry
- [ ] Context injection system
- [ ] Terminology cache
- [ ] Result merger with deduplication

### Phase 3: State Management (Week 3)
- [ ] Checkpoint system
- [ ] Progress tracking
- [ ] Session management

### Phase 4: Quality & Testing (Week 4)
- [ ] Validation layer
- [ ] Error handling & recovery
- [ ] Unit tests
- [ ] Integration tests
- [ ] Documentation

### Phase 5: Optimization (Week 5)
- [ ] Performance profiling
- [ ] Memory optimization
- [ ] Concurrent processing tuning
- [ ] Token budget optimization

---

## 10. Usage Example

Khi AI (Claude) sử dụng MCP server:

```
Human: Hãy dịch file medical_records.csv (100k rows) từ tiếng Việt sang tiếng Anh

Claude uses tools:
1. analyze_csv(file_path="medical_records.csv")
   → Returns: 100k rows, 15 columns, ~50MB, est. 20M tokens

2. configure_translation(
     source_lang="vi",
     target_lang="en", 
     columns_to_translate=["diagnosis", "symptoms", "treatment"],
     chunk_size=500,
     overlap_size=50,
     domain_context="medical",
     column_descriptions={
       "diagnosis": "Patient diagnosis in Vietnamese",
       "symptoms": "Reported symptoms",
       "treatment": "Prescribed treatment plan"
     }
   )

3. start_translation(
     input_file="medical_records.csv",
     output_file="medical_records_en.csv"
   )
   → Returns: session_id="abc-123", status="running"

4. [Periodically] get_translation_progress(session_id="abc-123")
   → Returns: progress=0.45, rows_processed=45000, estimated_time_remaining=300s

5. [On completion] 
   → Returns: status="completed", rows_processed=100000
```

---

## 11. Error Recovery Scenarios

### Scenario 1: API Rate Limit Hit
- **Detection**: HTTP 429 response
- **Action**: Exponential backoff (2s, 4s, 8s, 16s)
- **Fallback**: Reduce concurrent requests từ 5 → 2

### Scenario 2: Network Interruption
- **Detection**: Timeout hoặc connection error
- **Action**: Save checkpoint, log error
- **Recovery**: Resume từ last completed chunk

### Scenario 3: Translation Quality Issue
- **Detection**: Validation fails (length ratio >10x, all empty cells)
- **Action**: Retry with expanded context (+2 rows before/after)
- **Fallback**: Mark row for manual review, continue with next

### Scenario 4: Out of Memory
- **Detection**: Allocation error
- **Action**: Reduce chunk_size by 50%, clear terminology cache
- **Prevention**: Monitor memory usage, trigger GC if > 80MB

### Scenario 5: AI Hallucination (Row Count Mismatch)
- **Detection**: `original.rows.len() != translated.rows.len()`
- **Action**: Retry với temperature thấp hơn (0.1) và strict prompt
- **Fallback**: Giữ nguyên text gốc, log vào failed_rows.json để review
- **Prevention**: Thêm instruction rõ ràng trong prompt: "Output EXACTLY {N} rows"

### Scenario 6: Resequencer Buffer Overflow
- **Detection**: Buffer size >= max_buffer_size
- **Cause**: Một chunk bị treo (API timeout) trong khi các chunks sau đã hoàn thành
- **Action**: 
  1. Timeout chunk bị treo sau 5 phút
  2. Retry chunk đó với fresh connection
  3. Nếu vẫn fail, mark failed và continue
- **Prevention**: Giảm concurrency nếu thường xuyên xảy ra

---

## 12. Security Considerations

1. **API Key Storage**: Không hardcode, đọc từ environment variable hoặc config file với file permissions 600
2. **Input Validation**: Kiểm tra file path để tránh path traversal
3. **Resource Limits**: Giới hạn max file size (e.g., 10GB), max concurrent sessions
4. **Logging**: Không log sensitive data (API keys, personal info trong CSV)
5. **CSV Injection Protection**: Sanitize output để tránh Excel formula injection
   ```rust
   fn sanitize_cell(value: &str) -> String {
       if value.starts_with('=') || value.starts_with('+') || 
          value.starts_with('-') || value.starts_with('@') {
           format!("'{}", value)  // Prefix với single quote
       } else {
           value.to_string()
       }
   }
   ```

---

## 13. Monitoring & Observability

### Metrics to Track

```rust
use tracing::{info, warn, error};

// Log mỗi chunk completion
info!(
    session_id = %session.id,
    chunk_index = chunk.index,
    rows_processed = chunk.rows.len(),
    tokens_used = chunk.token_count,
    duration_ms = elapsed.as_millis(),
    "Chunk completed"
);

// Log errors with context
error!(
    session_id = %session.id,
    chunk_index = chunk.index,
    error = %e,
    "Translation failed"
);

// Log performance metrics
info!(
    throughput_rows_per_sec = throughput,
    memory_usage_mb = memory_mb,
    "Performance metrics"
);
```

### Health Checks

- **API connectivity**: Ping endpoint every 5 minutes
- **Disk space**: Check available space before writing
- **Memory usage**: Alert if > 90MB

---

## 14. Future Enhancements

1. **Batch mode**: Dịch nhiều files cùng lúc
2. **Smart resumption**: Nếu config thay đổi, tự động re-translate affected chunks
3. **Quality scoring**: Tính confidence score cho mỗi translation
4. **Terminology extraction**: Tự động học terminology từ context
5. **Parallel translation**: Dịch sang multiple languages cùng lúc
6. **Semantic chunking**: Dùng embeddings để detect topic boundaries
7. **UI dashboard**: Web interface để monitor progress

---

## 15. Deliverables

Sau khi triển khai xong, dự án sẽ bao gồm:

1. ✅ Source code (`src/` directory)
2. ✅ `Cargo.toml` với dependencies
3. ✅ `config.toml` mẫu
4. ✅ `README.md` với usage instructions
5. ✅ Tests (`tests/` directory)
6. ✅ Example CSV files (test data)
7. ✅ Claude Desktop config example
8. ✅ Performance benchmark results
9. ✅ API documentation (generated from code comments)

---

## Kết luận

Specification này cung cấp đầy đủ thông tin để AI triển khai MCP server. Các điểm quan trọng:

- ✅ **Không phình context**: Chunk size + overlap + context overhead < 10% max tokens (tiktoken-rs chính xác)
- ✅ **Không mất ngữ cảnh**: Overlap + context injection + terminology cache
- ✅ **Performance cao**: Rust async + streaming + zero-copy + bounded channels (backpressure)
- ✅ **Fault-tolerant**: Checkpoint + retry + validation + partial failure handling
- ✅ **Type-safe**: Rust type system + schema validation + redb
- ✅ **Concurrent-safe**: Resequencer đảm bảo thứ tự output khi xử lý parallel
- ✅ **Memory-safe**: Backpressure + buffer limits + session cleanup với TTL
- ✅ **AI Hallucination Handling**: Row count validation + fallback strategy

### Cải tiến so với bản gốc:
1. **redb** thay thế sled (maintain tốt hơn)
2. **tiktoken-rs** bắt buộc (ước lượng token chính xác)
3. **Resequencer** xử lý concurrent out-of-order
4. **Discard Edges** thay thế deduplicate (tránh string comparison)
5. **Session Cleanup** với TTL (tránh leak storage)
6. **Backpressure** với bounded channels (tránh OOM)
7. **Partial Failure Handling** (không fail toàn bộ vì 1 row)
8. **CSV Injection Protection** (security)

Bắt đầu từ Phase 1 và triển khai tuần tự. Good luck! 🚀

