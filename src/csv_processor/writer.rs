use crate::utils::{sanitize_cell, CsvTranslatorError, Result};
use csv::Writer;
use std::fs::File;
use tokio::sync::mpsc;

pub struct CsvStreamWriter {
    path: String,
    headers: Vec<String>,
    writer: Option<Writer<File>>,
    rows_written: usize,
}

impl CsvStreamWriter {
    pub fn new(path: impl Into<String>, headers: Vec<String>) -> Self {
        Self {
            path: path.into(),
            headers,
            writer: None,
            rows_written: 0,
        }
    }

    pub fn initialize(&mut self) -> Result<()> {
        let file = File::create(&self.path)?;
        let mut writer = Writer::from_writer(file);
        writer.write_record(&self.headers)?;
        self.writer = Some(writer);
        Ok(())
    }

    pub fn write_row(&mut self, row: &[String]) -> Result<()> {
        let writer = self.writer.as_mut().ok_or_else(|| {
            CsvTranslatorError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Writer not initialized",
            ))
        })?;

        let sanitized: Vec<String> = row.iter().map(|s| sanitize_cell(s)).collect();

        writer.write_record(&sanitized)?;
        self.rows_written += 1;
        Ok(())
    }

    pub fn write_rows(&mut self, rows: &[Vec<String>]) -> Result<()> {
        for row in rows {
            self.write_row(row)?;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer.flush()?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<usize> {
        if let Some(writer) = self.writer.take() {
            drop(writer);
        }
        Ok(self.rows_written)
    }

    pub fn rows_written(&self) -> usize {
        self.rows_written
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

pub struct AsyncCsvWriter {
    tx: mpsc::Sender<WriteCommand>,
}

enum WriteCommand {
    WriteRow(Vec<String>),
    WriteRows(Vec<Vec<String>>),
    Flush,
    Close,
}

impl AsyncCsvWriter {
    pub async fn new(path: String, headers: Vec<String>) -> Result<Self> {
        let (tx, mut rx) = mpsc::channel::<WriteCommand>(100);

        tokio::spawn(async move {
            let mut writer = CsvStreamWriter::new(path, headers);
            if let Err(e) = writer.initialize() {
                tracing::error!("Failed to initialize writer: {}", e);
                return;
            }

            while let Some(cmd) = rx.recv().await {
                match cmd {
                    WriteCommand::WriteRow(row) => {
                        if let Err(e) = writer.write_row(&row) {
                            tracing::error!("Failed to write row: {}", e);
                        }
                    }
                    WriteCommand::WriteRows(rows) => {
                        if let Err(e) = writer.write_rows(&rows) {
                            tracing::error!("Failed to write rows: {}", e);
                        }
                    }
                    WriteCommand::Flush => {
                        if let Err(e) = writer.flush() {
                            tracing::error!("Failed to flush: {}", e);
                        }
                    }
                    WriteCommand::Close => {
                        let _ = writer.finish();
                        break;
                    }
                }
            }
        });

        Ok(Self { tx })
    }

    pub async fn write_row(&self, row: Vec<String>) -> Result<()> {
        self.tx
            .send(WriteCommand::WriteRow(row))
            .await
            .map_err(|_| CsvTranslatorError::ChannelClosed)
    }

    pub async fn write_rows(&self, rows: Vec<Vec<String>>) -> Result<()> {
        self.tx
            .send(WriteCommand::WriteRows(rows))
            .await
            .map_err(|_| CsvTranslatorError::ChannelClosed)
    }

    pub async fn flush(&self) -> Result<()> {
        self.tx
            .send(WriteCommand::Flush)
            .await
            .map_err(|_| CsvTranslatorError::ChannelClosed)
    }

    pub async fn close(self) -> Result<()> {
        self.tx
            .send(WriteCommand::Close)
            .await
            .map_err(|_| CsvTranslatorError::ChannelClosed)
    }
}
