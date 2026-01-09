use crate::csv_processor::estimate_tokens;

pub struct LongCellHandler {
    max_tokens_per_cell: usize,
    #[allow(dead_code)]
    overlap_tokens: usize,
}

impl LongCellHandler {
    pub fn new(max_tokens_per_cell: usize, overlap_tokens: usize) -> Self {
        Self {
            max_tokens_per_cell,
            overlap_tokens,
        }
    }

    pub fn needs_chunking(&self, content: &str) -> bool {
        estimate_tokens(content) > self.max_tokens_per_cell
    }

    pub fn chunk_content(&self, content: &str) -> Vec<ContentChunk> {
        let tokens = estimate_tokens(content);

        if tokens <= self.max_tokens_per_cell {
            return vec![ContentChunk {
                content: content.to_string(),
                is_first: true,
                is_last: true,
                overlap_before: String::new(),
                overlap_after: String::new(),
            }];
        }

        let sentences: Vec<&str> = content
            .split(|c| c == '.' || c == '!' || c == '?')
            .filter(|s| !s.trim().is_empty())
            .collect();

        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        let mut current_tokens = 0;

        for (i, sentence) in sentences.iter().enumerate() {
            let sentence_with_punct = if i < sentences.len() - 1 {
                format!("{}. ", sentence.trim())
            } else {
                sentence.trim().to_string()
            };

            let sentence_tokens = estimate_tokens(&sentence_with_punct);

            if current_tokens + sentence_tokens > self.max_tokens_per_cell && !current_chunk.is_empty()
            {
                chunks.push(current_chunk.clone());
                current_chunk = String::new();
                current_tokens = 0;
            }

            current_chunk.push_str(&sentence_with_punct);
            current_tokens += sentence_tokens;
        }

        if !current_chunk.is_empty() {
            chunks.push(current_chunk);
        }

        chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| {
                let overlap_before = if i > 0 {
                    self.get_ending_overlap(&chunks[i - 1])
                } else {
                    String::new()
                };

                let overlap_after = if i < chunks.len() - 1 {
                    self.get_starting_overlap(&chunks[i + 1])
                } else {
                    String::new()
                };

                ContentChunk {
                    content: chunk.clone(),
                    is_first: i == 0,
                    is_last: i == chunks.len() - 1,
                    overlap_before,
                    overlap_after,
                }
            })
            .collect()
    }

    fn get_ending_overlap(&self, content: &str) -> String {
        let words: Vec<&str> = content.split_whitespace().collect();
        let overlap_word_count = 20;
        let start = words.len().saturating_sub(overlap_word_count);
        words[start..].join(" ")
    }

    fn get_starting_overlap(&self, content: &str) -> String {
        let words: Vec<&str> = content.split_whitespace().collect();
        let overlap_word_count = 20;
        let end = overlap_word_count.min(words.len());
        words[..end].join(" ")
    }

    pub fn merge_translations(&self, translated_chunks: Vec<String>) -> String {
        translated_chunks.join(" ")
    }
}

#[derive(Debug, Clone)]
pub struct ContentChunk {
    pub content: String,
    pub is_first: bool,
    pub is_last: bool,
    pub overlap_before: String,
    pub overlap_after: String,
}

impl ContentChunk {
    pub fn with_context(&self) -> String {
        let mut result = String::new();

        if !self.overlap_before.is_empty() {
            result.push_str("[Previous context: ");
            result.push_str(&self.overlap_before);
            result.push_str("]\n\n");
        }

        result.push_str(&self.content);

        if !self.overlap_after.is_empty() {
            result.push_str("\n\n[Following context: ");
            result.push_str(&self.overlap_after);
            result.push_str("]");
        }

        result
    }
}
