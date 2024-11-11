use thiserror::Error;
use winnow::error::{ContextError, ErrMode, ErrorKind, InputError};

pub type NixUriResult<T> = Result<T, NixUriError>;

#[derive(Debug, Error, PartialEq)]
#[non_exhaustive]
pub enum NixUriError {
    /// Generic nix uri error
    #[error("Error: {0}")]
    Error(String),
    /// Generic parsing fail
    #[error("Error parsing: {0}")]
    ParseError(String),
    /// Invalid Url
    #[error("Not a valid Url: {0}")]
    InvalidUrl(String),
    /// The path to directories must be absolute
    #[error("The path is not absolute: {0}")]
    NotAbsolute(String),
    /// Contained an Illegal Path Character
    #[error("Contains an illegal path character: {0}")]
    PathCharacter(String),
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
    UnknownTransportLayer(String),
    /// Invalid Type
    #[error("Invalid FlakeRef Type: {0}")]
    InvalidType(String),
    #[error("The parameter: {0} is not supported by the flakeref type.")]
    UnsupportedParam(String),
    #[error("field: `{0}` only supported by: `{1}`.")]
    UnsupportedByType(String, String),
    #[error("The parameter: {0} invalid.")]
    UnknownUriParameter(String),
    /// Nom Error
    /// TODO: Implement real conversion instead of this hack.
    #[error("Nom Error: {0}")]
    Nom(String),
    #[error("winnow parse error: {0}")]
    WinParseError(ErrMode<InputError<String>>),
    #[error("ctx error: {0}")]
    CtxError(ErrMode<ContextError>),
    #[error("Parser Error: {0}")]
    Parser(ErrMode<(String, ErrorKind)>),
    #[error("Servo Url Parsing Error: {0}")]
    ServoUrl(#[from] url::ParseError),
}

impl From<ErrMode<ContextError>> for NixUriError {
    fn from(value: ErrMode<ContextError>) -> Self {
        Self::CtxError(value)
    }
}
impl From<ErrMode<InputError<&str>>> for NixUriError {
    fn from(value: ErrMode<InputError<&str>>) -> Self {
        Self::WinParseError(value.map_input(str::to_string))
    }
}

impl From<ErrMode<(&str, ErrorKind)>> for NixUriError {
    fn from(value: ErrMode<(&str, ErrorKind)>) -> Self {
        Self::Parser(value.map(|(s, e)| (s.to_string(), e)))
    }
}
