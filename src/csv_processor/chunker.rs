use csv::StringRecord;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvChunk {
    pub index: usize,
    pub start_row: usize,
    pub end_row: usize,
    pub core_start: usize,
    pub core_end: usize,
    pub rows: Vec<Vec<String>>,
    pub headers: Vec<String>,
    pub is_first: bool,
    pub is_last: bool,
}

impl CsvChunk {
    pub fn core_rows(&self) -> &[Vec<String>] {
        let start = self.core_start - self.start_row;
        let end = self.core_end - self.start_row;
        &self.rows[start..end]
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn core_row_count(&self) -> usize {
        self.core_end - self.core_start
    }
}

pub struct CsvChunker {
    chunk_size: usize,
    overlap_size: usize,
}

impl CsvChunker {
    pub fn new(chunk_size: usize, overlap_size: usize) -> Self {
        Self {
            chunk_size,
            overlap_size,
        }
    }

    pub fn calculate_chunks(&self, total_rows: usize) -> Vec<ChunkRange> {
        let mut chunks = Vec::new();
        let mut current_start = 0;
        let mut chunk_index = 0;

        while current_start < total_rows {
            let is_first = chunk_index == 0;

            let overlap_before = if is_first { 0 } else { self.overlap_size };
            let actual_start = current_start.saturating_sub(overlap_before);

            let core_end = (current_start + self.chunk_size).min(total_rows);
            let is_last = core_end >= total_rows;

            let overlap_after = if is_last { 0 } else { self.overlap_size };
            let actual_end = (core_end + overlap_after).min(total_rows);

            chunks.push(ChunkRange {
                index: chunk_index,
                start_row: actual_start,
                end_row: actual_end,
                core_start: current_start,
                core_end,
                is_first,
                is_last,
            });

            current_start = core_end;
            chunk_index += 1;
        }

        chunks
    }

    pub fn create_chunk(
        &self,
        range: &ChunkRange,
        rows: Vec<Vec<String>>,
        headers: Vec<String>,
    ) -> CsvChunk {
        CsvChunk {
            index: range.index,
            start_row: range.start_row,
            end_row: range.end_row,
            core_start: range.core_start,
            core_end: range.core_end,
            rows,
            headers,
            is_first: range.is_first,
            is_last: range.is_last,
        }
    }

    pub fn chunk_size(&self) -> usize {
        self.chunk_size
    }

    pub fn overlap_size(&self) -> usize {
        self.overlap_size
    }
}

#[derive(Debug, Clone)]
pub struct ChunkRange {
    pub index: usize,
    pub start_row: usize,
    pub end_row: usize,
    pub core_start: usize,
    pub core_end: usize,
    pub is_first: bool,
    pub is_last: bool,
}

impl ChunkRange {
    pub fn row_count(&self) -> usize {
        self.end_row - self.start_row
    }

    pub fn core_row_count(&self) -> usize {
        self.core_end - self.core_start
    }
}

pub fn string_record_to_vec(record: &StringRecord) -> Vec<String> {
    record.iter().map(|s| s.to_string()).collect()
}
