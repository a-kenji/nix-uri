use nom::error::{ContextError, ErrorKind, FromExternalError, ParseError};
use std::fmt;
use thiserror::Error;

pub type NixUriResult<T> = Result<T, NixUriError>;

/// Custom error tree type compatible with nom 8.0
/// This provides similar functionality to nom-supreme's ErrorTree
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorTree<I> {
    /// A base error - a leaf node in the error tree
    Base {
        /// The location in the input where the error occurred
        location: I,
        /// The kind of error that occurred
        kind: BaseErrorKind,
    },
    /// A stack of errors with context
    Stack {
        /// The base error
        base: Box<ErrorTree<I>>,
        /// The context stack
        contexts: Vec<(I, StackContext)>,
    },
    /// A collection of alternative errors from `alt`
    Alt(Vec<ErrorTree<I>>),
}

/// The kind of base error
#[derive(Debug, Clone, PartialEq)]
pub enum BaseErrorKind {
    /// A nom ErrorKind
    Kind(ErrorKind),
    /// An expected value
    Expected(Expectation),
    /// An external error
    External(String),
}

/// What was expected when parsing
#[derive(Debug, Clone, PartialEq)]
pub enum Expectation {
    /// Expected a specific tag
    Tag(&'static str),
    /// Expected a specific character
    Char(char),
    /// Expected end of input
    Eof,
    /// Expected alpha character
    Alpha,
    /// Expected digit
    Digit,
    /// Expected hex digit
    HexDigit,
    /// Expected alphanumeric
    AlphaNumeric,
    /// Expected space
    Space,
    /// Expected multispace
    Multispace,
    /// Expected something else
    Something,
}

/// Context added to a stack
#[derive(Debug, Clone, PartialEq)]
pub enum StackContext {
    /// A static context string
    Context(&'static str),
    /// A nom ErrorKind
    Kind(ErrorKind),
}

impl<I> ErrorTree<I> {
    /// Map the locations in this error tree
    pub fn map_locations<O>(self, f: impl Fn(I) -> O + Copy) -> ErrorTree<O> {
        match self {
            ErrorTree::Base { location, kind } => ErrorTree::Base {
                location: f(location),
                kind,
            },
            ErrorTree::Stack { base, contexts } => ErrorTree::Stack {
                base: Box::new(base.map_locations(f)),
                contexts: contexts.into_iter().map(|(i, c)| (f(i), c)).collect(),
            },
            ErrorTree::Alt(alts) => {
                ErrorTree::Alt(alts.into_iter().map(|e| e.map_locations(f)).collect())
            }
        }
    }
}

impl<I: Clone> ParseError<I> for ErrorTree<I> {
    fn from_error_kind(input: I, kind: ErrorKind) -> Self {
        ErrorTree::Base {
            location: input,
            kind: BaseErrorKind::Kind(kind),
        }
    }

    fn append(input: I, kind: ErrorKind, other: Self) -> Self {
        let context = (input, StackContext::Kind(kind));
        match other {
            ErrorTree::Stack { base, mut contexts } => {
                contexts.push(context);
                ErrorTree::Stack { base, contexts }
            }
            base => ErrorTree::Stack {
                base: Box::new(base),
                contexts: vec![context],
            },
        }
    }

    fn from_char(input: I, c: char) -> Self {
        ErrorTree::Base {
            location: input,
            kind: BaseErrorKind::Expected(Expectation::Char(c)),
        }
    }

    fn or(self, other: Self) -> Self {
        // Combine alternatives
        let mut alts = match self {
            ErrorTree::Alt(v) => v,
            e => vec![e],
        };
        match other {
            ErrorTree::Alt(v) => alts.extend(v),
            e => alts.push(e),
        }
        ErrorTree::Alt(alts)
    }
}

impl<I: Clone> ContextError<I> for ErrorTree<I> {
    fn add_context(input: I, ctx: &'static str, other: Self) -> Self {
        let context = (input, StackContext::Context(ctx));
        match other {
            ErrorTree::Stack { base, mut contexts } => {
                contexts.push(context);
                ErrorTree::Stack { base, contexts }
            }
            base => ErrorTree::Stack {
                base: Box::new(base),
                contexts: vec![context],
            },
        }
    }
}

impl<I: Clone, E: std::fmt::Display> FromExternalError<I, E> for ErrorTree<I> {
    fn from_external_error(input: I, _kind: ErrorKind, e: E) -> Self {
        ErrorTree::Base {
            location: input,
            kind: BaseErrorKind::External(e.to_string()),
        }
    }
}

impl<I: fmt::Display> fmt::Display for ErrorTree<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorTree::Base { location, kind } => {
                write!(f, "error {:?} at: {}", kind, location)
            }
            ErrorTree::Stack { base, contexts } => {
                write!(f, "{}", base)?;
                for (input, context) in contexts {
                    write!(f, "\n  in {:?} at: {}", context, input)?;
                }
                Ok(())
            }
            ErrorTree::Alt(alts) => {
                writeln!(f, "one of:")?;
                for alt in alts {
                    writeln!(f, "  {}", alt)?;
                }
                Ok(())
            }
        }
    }
}

impl<I: fmt::Debug + fmt::Display> std::error::Error for ErrorTree<I> {}

pub type IErr<E> = ErrorTree<E>;

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
    NomParseError(#[from] IErr<String>),
    // #[error("{} {}", 0.0, 0.1)]
    // Parser((String, VerboseErrorKind)),
    #[error("Servo Url Parsing Error: {0}")]
    ServoUrl(#[from] url::ParseError),
}

impl From<IErr<&str>> for NixUriError {
    fn from(value: IErr<&str>) -> Self {
        let new_errs = value.map_locations(|i| i.to_string());
        Self::NomParseError(new_errs)
    }
}

/// A tag combinator that produces nice error messages with `Expected(Tag(...))`
/// This is similar to nom-supreme's tag functionality
pub fn tag<'a>(
    expected: &'static str,
) -> impl FnMut(&'a str) -> nom::IResult<&'a str, &'a str, ErrorTree<&'a str>> {
    move |input: &'a str| {
        if let Some(rest) = input.strip_prefix(expected) {
            Ok((rest, &input[..expected.len()]))
        } else {
            Err(nom::Err::Error(ErrorTree::Base {
                location: input,
                kind: BaseErrorKind::Expected(Expectation::Tag(expected)),
            }))
        }
    }
}
