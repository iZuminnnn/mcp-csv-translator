use crate::translation::client::TranslationContext;
use crate::utils::TranslationSessionConfig;
use std::collections::HashMap;

pub struct ContextManager {
    #[allow(dead_code)]
    config: TranslationSessionConfig,
    terminology: HashMap<String, String>,
    file_summary: Option<String>,
    previous_chunk_ending: Vec<Vec<String>>,
}

impl ContextManager {
    pub fn new(config: TranslationSessionConfig) -> Self {
        Self {
            config,
            terminology: HashMap::new(),
            file_summary: None,
            previous_chunk_ending: Vec::new(),
        }
    }

    pub fn build_context(&self) -> TranslationContext {
        let mut context = TranslationContext::new();

        for (source, target) in &self.terminology {
            context.add_terminology(source.clone(), target.clone());
        }

        if !self.previous_chunk_ending.is_empty() {
            context.set_previous_rows(self.previous_chunk_ending.clone());
        }

        if let Some(summary) = &self.file_summary {
            context.set_file_summary(summary.clone());
        }

        context
    }

    pub fn update_from_translation(&mut self, translated_rows: &[Vec<String>], ending_rows: usize) {
        let start = translated_rows.len().saturating_sub(ending_rows);
        self.previous_chunk_ending = translated_rows[start..].to_vec();
    }

    pub fn add_terminology(&mut self, source: String, target: String) {
        self.terminology.insert(source, target);
    }

    pub fn add_terminologies(&mut self, terms: HashMap<String, String>) {
        self.terminology.extend(terms);
    }

    pub fn set_file_summary(&mut self, summary: String) {
        self.file_summary = Some(summary);
    }

    pub fn terminology(&self) -> &HashMap<String, String> {
        &self.terminology
    }

    pub fn extract_terminology_from_translation(
        original: &[Vec<String>],
        translated: &[Vec<String>],
        columns_to_translate: &[String],
        headers: &[String],
    ) -> HashMap<String, String> {
        let mut terms = HashMap::new();

        let col_indices: Vec<usize> = columns_to_translate
            .iter()
            .filter_map(|col| headers.iter().position(|h| h == col))
            .collect();

        for (orig_row, trans_row) in original.iter().zip(translated.iter()) {
            for &idx in &col_indices {
                if let (Some(orig), Some(trans)) = (orig_row.get(idx), trans_row.get(idx)) {
                    if !orig.is_empty() && !trans.is_empty() && orig != trans {
                        let orig_words: Vec<&str> = orig.split_whitespace().collect();
                        let trans_words: Vec<&str> = trans.split_whitespace().collect();

                        if orig_words.len() <= 3 && trans_words.len() <= 5 {
                            terms.insert(orig.clone(), trans.clone());
                        }
                    }
                }
            }
        }

        terms
    }
}

pub fn build_system_prompt(config: &TranslationSessionConfig) -> String {
    let mut prompt = format!(
        "You are an expert translator specializing in {} to {} translation.",
        config.source_lang, config.target_lang
    );

    if let Some(domain) = &config.domain_context {
        prompt.push_str(&format!(
            " You have deep expertise in {} domain terminology.",
            domain
        ));
    }

    prompt.push_str("\n\nKey instructions:\n");
    prompt.push_str("1. Maintain consistent terminology throughout the translation\n");
    prompt.push_str("2. Preserve the exact CSV structure - same number of rows and columns\n");
    prompt.push_str("3. Only translate the specified columns, leave others unchanged\n");
    prompt.push_str("4. Preserve any special characters, numbers, or codes\n");
    prompt.push_str("5. Maintain the same register and tone as the original\n");

    prompt
}
