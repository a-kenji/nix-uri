use thiserror::Error;

#[derive(Debug, Error)]
pub enum FlkError {
    /// Io Error
    #[error("IoError: {0}")]
    Io(#[from] std::io::Error),
    #[error("DataError: {0}")]
    DataError(#[from] flk_data::errors::DataError),
    // Deserialization Error
    #[error("Deserialization Error: {0}")]
    Serde(#[from] serde_json::Error),
    // /// Deserialization Toml Error
    // #[error("Deserialization Toml Error: {0}")]
    // SerdeToml(#[from] toml::de::Error),
    // #[error("Utf8 Conversion Error")]
    // Utf8(#[from] std::str::Utf8Error),
    // Reqwest Error
    #[error("Reqwest Error")]
    Reqwest(#[from] reqwest::Error),
    // Reqwest Error
    // #[error("Incorrect Channel")]
    // IncorrectChannel(String),
}
