use crate::csv_processor::CsvChunk;
use crate::utils::{CsvTranslatorError, Result, TranslationSessionConfig};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::warn;

pub struct TranslationClient {
    client: Client,
    config: TranslationSessionConfig,
    max_retries: usize,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: usize,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

impl TranslationClient {
    pub fn new(config: TranslationSessionConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            config,
            max_retries: 3,
        }
    }

    pub async fn translate_chunk(
        &self,
        chunk: &CsvChunk,
        context: &TranslationContext,
    ) -> Result<Vec<Vec<String>>> {
        let prompt = self.build_translation_prompt(chunk, context);

        let mut last_error = None;
        let mut temperature = 0.3;

        for attempt in 0..self.max_retries {
            match self.call_api(&prompt, temperature).await {
                Ok(response) => {
                    match self.parse_translation_response(&response, chunk.row_count()) {
                        Ok(translated_rows) => {
                            if translated_rows.len() == chunk.row_count() {
                                return Ok(translated_rows);
                            } else {
                                warn!(
                                    expected = chunk.row_count(),
                                    got = translated_rows.len(),
                                    "Row count mismatch, retrying with lower temperature"
                                );
                                temperature = 0.1;
                                last_error = Some(CsvTranslatorError::RowCountMismatch {
                                    expected: chunk.row_count(),
                                    got: translated_rows.len(),
                                });
                            }
                        }
                        Err(e) => {
                            last_error = Some(e);
                        }
                    }
                }
                Err(e) => {
                    warn!(attempt = attempt, error = %e, "Translation API call failed");
                    last_error = Some(e);

                    if attempt < self.max_retries - 1 {
                        let delay = Duration::from_secs(2u64.pow(attempt as u32));
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            CsvTranslatorError::TranslationFailed("Unknown error".to_string())
        }))
    }

    fn build_translation_prompt(&self, chunk: &CsvChunk, context: &TranslationContext) -> String {
        let mut prompt = String::new();

        prompt.push_str(&format!(
            "You are a professional translator. Translate the following CSV data from {} to {}.\n\n",
            self.config.source_lang, self.config.target_lang
        ));

        if let Some(domain) = &self.config.domain_context {
            prompt.push_str(&format!("Domain context: {}\n", domain));
        }

        if let Some(descriptions) = &self.config.column_descriptions {
            prompt.push_str("\nColumn descriptions:\n");
            for (col, desc) in descriptions {
                prompt.push_str(&format!("- {}: {}\n", col, desc));
            }
        }

        if !context.terminology.is_empty() {
            prompt.push_str("\nTerminology to use consistently:\n");
            for (source, target) in &context.terminology {
                prompt.push_str(&format!("- \"{}\" → \"{}\"\n", source, target));
            }
        }

        prompt.push_str(&format!(
            "\nIMPORTANT: You MUST output EXACTLY {} rows. Do not add or remove any rows.\n",
            chunk.row_count()
        ));

        prompt.push_str("\nColumns to translate: ");
        prompt.push_str(&self.config.columns_to_translate.join(", "));
        prompt.push_str("\n\nInput data (CSV format):\n");

        prompt.push_str(&chunk.headers.join(","));
        prompt.push('\n');

        for row in &chunk.rows {
            prompt.push_str(&row.join(","));
            prompt.push('\n');
        }

        prompt.push_str("\nOutput the translated CSV with the same structure. Only translate the specified columns, keep other columns unchanged.\n");
        prompt.push_str("Output ONLY the CSV data, no explanations or markdown formatting.\n");

        prompt
    }

    async fn call_api(&self, prompt: &str, temperature: f32) -> Result<String> {
        let request = AnthropicRequest {
            model: self.config.model.clone(),
            max_tokens: 8192,
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            temperature: Some(temperature),
        };

        let response = self
            .client
            .post(&self.config.api_endpoint)
            .header("Content-Type", "application/json")
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(CsvTranslatorError::ApiError(format!(
                "API returned {}: {}",
                status, body
            )));
        }

        let api_response: AnthropicResponse = response.json().await?;

        api_response
            .content
            .into_iter()
            .find_map(|block| {
                if block.content_type == "text" {
                    block.text
                } else {
                    None
                }
            })
            .ok_or_else(|| CsvTranslatorError::ApiError("No text content in response".to_string()))
    }

    fn parse_translation_response(
        &self,
        response: &str,
        expected_rows: usize,
    ) -> Result<Vec<Vec<String>>> {
        let mut rows = Vec::new();
        let lines: Vec<&str> = response.lines().filter(|l| !l.trim().is_empty()).collect();

        let start_idx = if lines.first().map(|l| l.contains(",")).unwrap_or(false)
            && lines.len() > expected_rows
        {
            1
        } else {
            0
        };

        for line in lines.into_iter().skip(start_idx) {
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(line.as_bytes());

            if let Some(record) = reader.records().next() {
                match record {
                    Ok(r) => {
                        rows.push(r.iter().map(|s| s.to_string()).collect());
                    }
                    Err(_) => {
                        rows.push(vec![line.to_string()]);
                    }
                }
            }
        }

        Ok(rows)
    }
}

#[derive(Debug, Clone, Default)]
pub struct TranslationContext {
    pub terminology: std::collections::HashMap<String, String>,
    pub previous_rows: Vec<Vec<String>>,
    pub file_summary: Option<String>,
}

impl TranslationContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_terminology(&mut self, source: String, target: String) {
        self.terminology.insert(source, target);
    }

    pub fn set_previous_rows(&mut self, rows: Vec<Vec<String>>) {
        self.previous_rows = rows;
    }

    pub fn set_file_summary(&mut self, summary: String) {
        self.file_summary = Some(summary);
    }
}
