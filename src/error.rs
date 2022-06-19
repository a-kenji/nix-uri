use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NixUriError {
    /// Generic parsing fail
    #[error("Error parsing: {0}")]
    ParseError(String),
    /// The path to directories must be absolute
    #[error("The path is not absolute.")]
    NotAbsolute,
    #[error("The type is not known: {0}")]
    UnknownUrlType(String),
    /// Invalid Type
    #[error("Invalid FlakeRef Type: {0}")]
    InvalidType(String),
    #[error("The parameter: {0} is not supported by the flakeref type.")]
    UnsupportedParam(String),
    /// Io Error
    #[error("IoError: {0}")]
    Io(#[from] std::io::Error),
    /// Nom Error
    /// TODO: Implement real conversion instead of this hack.
    #[error("Nom Error: {0}")]
    Nom(String),
    #[error(transparent)]
    NomParseError(#[from] nom::Err<nom::error::Error<String>>),
    #[error(transparent)]
    Parser(#[from] nom::Err<(String, nom::error::ErrorKind)>),
}

impl From<nom::Err<nom::error::Error<&str>>> for NixUriError {
    fn from(value: nom::Err<nom::error::Error<&str>>) -> Self {
        Self::NomParseError(value.to_owned())
    }
}

impl From<nom::Err<(&str, nom::error::ErrorKind)>> for NixUriError {
    fn from(value: nom::Err<(&str, nom::error::ErrorKind)>) -> Self {
        Self::Parser(value.to_owned())
    }
}
