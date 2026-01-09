use crate::csv_processor::reader::{get_file_size, CsvStreamReader};
use crate::utils::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tiktoken_rs::cl100k_base;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvMetadata {
    pub total_rows: usize,
    pub total_columns: usize,
    pub column_names: Vec<String>,
    pub file_size_bytes: u64,
    pub estimated_tokens: usize,
    pub sample_data: Vec<JsonValue>,
}

pub async fn analyze_csv(file_path: &str, sample_rows: usize) -> Result<CsvMetadata> {
    let mut reader = CsvStreamReader::new(file_path);

    let headers = reader.read_headers().await?;
    let column_names: Vec<String> = headers.iter().map(|s| s.to_string()).collect();
    let total_columns = column_names.len();

    let total_rows = reader.count_rows().await?;

    let file_size_bytes = get_file_size(file_path).await?;

    let sample_records = reader.read_sample_rows(sample_rows).await?;
    let sample_data: Vec<JsonValue> = sample_records
        .iter()
        .map(|record| {
            let mut map = serde_json::Map::new();
            for (i, field) in record.iter().enumerate() {
                if let Some(col_name) = column_names.get(i) {
                    map.insert(col_name.clone(), JsonValue::String(field.to_string()));
                }
            }
            JsonValue::Object(map)
        })
        .collect();

    let estimated_tokens = estimate_tokens_for_csv(&sample_records, total_rows, &column_names);

    Ok(CsvMetadata {
        total_rows,
        total_columns,
        column_names,
        file_size_bytes,
        estimated_tokens,
        sample_data,
    })
}

pub fn estimate_tokens_for_csv(
    sample_records: &[csv::StringRecord],
    total_rows: usize,
    column_names: &[String],
) -> usize {
    if sample_records.is_empty() || total_rows == 0 {
        return 0;
    }

    let bpe = match cl100k_base() {
        Ok(b) => b,
        Err(_) => return estimate_tokens_fallback(sample_records, total_rows),
    };

    let sample_text: String = sample_records
        .iter()
        .flat_map(|record| record.iter())
        .collect::<Vec<_>>()
        .join(" ");

    let sample_tokens = bpe.encode_with_special_tokens(&sample_text).len();

    let avg_tokens_per_row = sample_tokens as f64 / sample_records.len() as f64;

    let header_tokens = bpe
        .encode_with_special_tokens(&column_names.join(" "))
        .len();

    let estimated = (avg_tokens_per_row * total_rows as f64) as usize + header_tokens;

    (estimated as f64 * 1.1) as usize
}

fn estimate_tokens_fallback(sample_records: &[csv::StringRecord], total_rows: usize) -> usize {
    let sample_chars: usize = sample_records
        .iter()
        .flat_map(|record| record.iter())
        .map(|s| s.len())
        .sum();

    let avg_chars_per_row = sample_chars as f64 / sample_records.len() as f64;
    let total_chars = (avg_chars_per_row * total_rows as f64) as usize;

    total_chars / 4
}

pub fn estimate_tokens(text: &str) -> usize {
    match cl100k_base() {
        Ok(bpe) => bpe.encode_with_special_tokens(text).len(),
        Err(_) => text.len() / 4,
    }
}

pub fn estimate_chunk_tokens(rows: &[Vec<String>], headers: &[String]) -> usize {
    let bpe = match cl100k_base() {
        Ok(b) => b,
        Err(_) => {
            let total_chars: usize = rows
                .iter()
                .flat_map(|row| row.iter())
                .map(|s| s.len())
                .sum();
            return total_chars / 4;
        }
    };

    let header_text = headers.join(" ");
    let mut total_tokens = bpe.encode_with_special_tokens(&header_text).len();

    for row in rows {
        let row_text = row.join(" ");
        total_tokens += bpe.encode_with_special_tokens(&row_text).len();
    }

    total_tokens
}
