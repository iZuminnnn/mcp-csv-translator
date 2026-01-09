use crate::csv_processor::chunker::CsvChunk;
use crate::utils::{CsvTranslatorError, Result};
use std::collections::BTreeMap;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct TranslatedChunk {
    pub index: usize,
    pub core_rows: Vec<Vec<String>>,
    pub core_start: usize,
    pub core_end: usize,
}

pub struct Resequencer {
    buffer: BTreeMap<usize, TranslatedChunk>,
    next_expected: usize,
    max_buffer_size: usize,
    output_tx: mpsc::Sender<TranslatedChunk>,
}

impl Resequencer {
    pub fn new(max_buffer_size: usize, output_tx: mpsc::Sender<TranslatedChunk>) -> Self {
        Self {
            buffer: BTreeMap::new(),
            next_expected: 0,
            max_buffer_size,
            output_tx,
        }
    }

    pub async fn add_chunk(&mut self, chunk: TranslatedChunk) -> Result<()> {
        if self.buffer.len() >= self.max_buffer_size {
            return Err(CsvTranslatorError::ResequencerOverflow {
                waiting_for: self.next_expected,
            });
        }

        self.buffer.insert(chunk.index, chunk);

        self.flush_ready().await?;

        Ok(())
    }

    async fn flush_ready(&mut self) -> Result<()> {
        while let Some(chunk) = self.buffer.remove(&self.next_expected) {
            self.output_tx
                .send(chunk)
                .await
                .map_err(|_| CsvTranslatorError::ChannelClosed)?;
            self.next_expected += 1;
        }
        Ok(())
    }

    pub fn pending_count(&self) -> usize {
        self.buffer.len()
    }

    pub fn next_expected(&self) -> usize {
        self.next_expected
    }

    pub fn is_complete(&self, total_chunks: usize) -> bool {
        self.next_expected >= total_chunks && self.buffer.is_empty()
    }
}

pub struct ResequencerHandle {
    input_tx: mpsc::Sender<TranslatedChunk>,
    output_rx: mpsc::Receiver<TranslatedChunk>,
}

impl ResequencerHandle {
    pub fn new(max_buffer_size: usize, channel_size: usize) -> Self {
        let (input_tx, mut input_rx) = mpsc::channel::<TranslatedChunk>(channel_size);
        let (output_tx, output_rx) = mpsc::channel::<TranslatedChunk>(channel_size);

        tokio::spawn(async move {
            let mut resequencer = Resequencer::new(max_buffer_size, output_tx);

            while let Some(chunk) = input_rx.recv().await {
                if let Err(e) = resequencer.add_chunk(chunk).await {
                    tracing::error!("Resequencer error: {}", e);
                    break;
                }
            }
        });

        Self {
            input_tx,
            output_rx,
        }
    }

    pub async fn send(&self, chunk: TranslatedChunk) -> Result<()> {
        self.input_tx
            .send(chunk)
            .await
            .map_err(|_| CsvTranslatorError::ChannelClosed)
    }

    pub async fn recv(&mut self) -> Option<TranslatedChunk> {
        self.output_rx.recv().await
    }

    pub fn sender(&self) -> mpsc::Sender<TranslatedChunk> {
        self.input_tx.clone()
    }
}

pub fn extract_core_rows(chunk: &CsvChunk, translated_rows: Vec<Vec<String>>) -> Vec<Vec<String>> {
    let start_offset = chunk.core_start - chunk.start_row;
    let end_offset = chunk.core_end - chunk.start_row;

    if end_offset <= translated_rows.len() {
        translated_rows[start_offset..end_offset].to_vec()
    } else {
        tracing::warn!(
            "Translated rows count mismatch: expected at least {}, got {}",
            end_offset,
            translated_rows.len()
        );
        translated_rows
            .into_iter()
            .skip(start_offset)
            .take(chunk.core_end - chunk.core_start)
            .collect()
    }
}
