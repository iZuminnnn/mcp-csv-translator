use crate::utils::{CsvTranslatorError, Result};
use csv::StringRecord;
use std::path::Path;

pub struct CsvStreamReader {
    path: String,
    headers: Option<StringRecord>,
}

impl CsvStreamReader {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            headers: None,
        }
    }

    pub async fn read_headers(&mut self) -> Result<StringRecord> {
        let file = std::fs::File::open(&self.path)?;
        let mut reader = csv::Reader::from_reader(file);
        let headers = reader
            .headers()
            .map_err(|e| CsvTranslatorError::CsvError(e))?
            .clone();
        self.headers = Some(headers.clone());
        Ok(headers)
    }

    pub fn headers(&self) -> Option<&StringRecord> {
        self.headers.as_ref()
    }

    pub async fn count_rows(&self) -> Result<usize> {
        let file = std::fs::File::open(&self.path)?;
        let mut reader = csv::Reader::from_reader(file);
        let count = reader.records().count();
        Ok(count)
    }

    pub async fn read_sample_rows(&self, n: usize) -> Result<Vec<StringRecord>> {
        let file = std::fs::File::open(&self.path)?;
        let mut reader = csv::Reader::from_reader(file);
        let mut rows = Vec::with_capacity(n);

        for result in reader.records().take(n) {
            let record = result?;
            rows.push(record);
        }

        Ok(rows)
    }

    pub fn iter_records(&self) -> Result<CsvRecordIterator> {
        let file = std::fs::File::open(&self.path)?;
        let reader = csv::Reader::from_reader(file);
        Ok(CsvRecordIterator {
            reader,
            current_index: 0,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

pub struct CsvRecordIterator {
    reader: csv::Reader<std::fs::File>,
    current_index: usize,
}

impl Iterator for CsvRecordIterator {
    type Item = Result<(usize, StringRecord)>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.reader.records().next() {
            Some(Ok(record)) => {
                let index = self.current_index;
                self.current_index += 1;
                Some(Ok((index, record)))
            }
            Some(Err(e)) => Some(Err(CsvTranslatorError::CsvError(e))),
            None => None,
        }
    }
}

pub async fn get_file_size(path: &str) -> Result<u64> {
    let metadata = tokio::fs::metadata(path).await?;
    Ok(metadata.len())
}

pub fn file_exists(path: &str) -> bool {
    Path::new(path).exists()
}
