use thiserror::Error;

pub(crate) type NixUriResult<T> = Result<T, NixUriError>;

#[derive(Debug, Error, PartialEq)]
#[non_exhaustive]
pub enum NixUriError {
    /// Generic parsing fail
    #[error("Error parsing: {0}")]
    ParseError(String),
    /// The path to directories must be absolute
    #[error("The path is not absolute.")]
    NotAbsolute,
    /// The type doesn't have the required default parameter set
    /// Example: Github needs to have an owner and a repo
    // TODO collect multiple potentially missing parameters
    #[error("FlakeRef Type: {0} is missing the following required parameter: {1}")]
    MissingTypeParameter(String, String),
    /// The type of the uri itself, for example `github`
    #[error("The type is not known: {0}")]
    UnknownUriType(String),
    /// The type of the uri extensions for a uri type, for example `git+ssh`
    /// the ssh part is the type here.
    #[error("The type is not known: {0}")]
    UnknownUrlType(String),
    /// Invalid Type
    #[error("Invalid FlakeRef Type: {0}")]
    InvalidType(String),
    #[error("The parameter: {0} is not supported by the flakeref type.")]
    UnsupportedParam(String),
    #[error("The parameter: {0} invalid.")]
    UnknownUriParameter(String),
    /// Nom Error
    /// TODO: Implement real conversion instead of this hack.
    #[error("Nom Error: {0}")]
    Nom(String),
    #[error(transparent)]
    NomParseError(#[from] nom::Err<nom::error::Error<String>>),
    #[error(transparent)]
    Parser(#[from] nom::Err<(String, nom::error::ErrorKind)>),
    #[cfg(feature = "url")]
    #[error("Error parsing the following url: {0}")]
    Url(#[from] url::ParseError),
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
