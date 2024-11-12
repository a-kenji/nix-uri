use nom_supreme::error::ErrorTree;
use thiserror::Error;

pub type NixUriResult<T> = Result<T, NixUriError>;

#[derive(Debug, Error)]
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
    #[error(transparent)]
    NomParseError(#[from] ErrorTree<String>),
    // #[error("{} {}", 0.0, 0.1)]
    // Parser((String, VerboseErrorKind)),
    #[error("Servo Url Parsing Error: {0}")]
    ServoUrl(#[from] url::ParseError),
}

impl From<ErrorTree<&str>> for NixUriError {
    fn from(value: ErrorTree<&str>) -> Self {
        let new_errs = value.map_locations(|i| i.to_string());
        Self::NomParseError(new_errs)
    }
}

// impl From<(&str, VerboseErrorKind)> for NixUriError {
//     fn from(value: (&str, VerboseErrorKind)) -> Self {
//         Self::Parser((value.0.to_string(), value.1))
//     }
// }
