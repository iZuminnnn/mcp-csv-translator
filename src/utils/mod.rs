pub mod config;
pub mod errors;

pub use config::{AppConfig, TranslationSessionConfig};
pub use errors::{CsvTranslatorError, Result, RetryStrategy};

pub fn sanitize_cell(value: &str) -> String {
    if value.starts_with('=')
        || value.starts_with('+')
        || value.starts_with('-')
        || value.starts_with('@')
    {
        format!("'{}", value)
    } else {
        value.to_string()
    }
}
