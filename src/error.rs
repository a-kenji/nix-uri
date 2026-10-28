use std::fmt;

use thiserror::Error;

pub type NixUriResult<T> = Result<T, NixUriError>;

/// Failures the parser may produce.
///
/// Variants are stable categories: callers can match `Parse` for a syntactic
/// failure with a byte position, `Unsupported` for a recognised input that
/// asks for something the library does not implement, `InvalidUrl` for input
/// that fails the high-level URL shape, and `InvalidValue` for a field that
/// passed parsing but failed validation. Structural failures that involve a
/// relationship between fields (rather than a single value's literal shape)
/// surface as named variants such as `FieldConflict`, `DuplicateField`, or
/// `TooManyIndirectSegments`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NixUriError {
    /// Parser failed at the given byte offset, expecting `expected`.
    #[error("parse error at byte {position}: expected {expected}")]
    Parse {
        position: usize,
        expected: ParseExpected,
    },
    /// The input parsed but requests an unsupported operation, parameter,
    /// type, or transport layer.
    #[error("{0}")]
    Unsupported(UnsupportedReason),
    /// The input is malformed at a higher level than the byte parser:
    /// an illegal path character, a non-absolute path where one was required,
    /// or an otherwise unparseable URL shape.
    #[error("not a valid URL: {0}")]
    InvalidUrl(String),
    /// A field's value did not pass validation. Reserved for failures where
    /// a single field's literal shape is wrong (e.g. a `?rev=` that is not
    /// 40 hex chars). Failures that involve a relationship between two
    /// fields surface as [`Self::FieldConflict`].
    #[error("invalid value for `{field}`: {reason}")]
    InvalidValue { field: &'static str, reason: String },
    /// Two fields were populated that the grammar treats as mutually
    /// exclusive. The canonical case is a `GitForge` input that supplies
    /// both `ref` and `rev`; Nix rejects the same shape.
    #[error("`{left}` and `{right}` are mutually exclusive")]
    FieldConflict {
        left: &'static str,
        right: &'static str,
    },
    /// A field was given twice, for example a git-forge ref both as a
    /// path segment and as `?ref=` (`github:o/r/main?ref=dev`).
    #[error("`{field}` is given more than once")]
    DuplicateField { field: &'static str },
    /// An indirect reference (a `flake:` URL or a bare flake id) exceeded
    /// the three-segment cap (`id[/ref[/rev]]`). `count` is the number
    /// of `/`-separated segments, including the id segment.
    #[error("indirect form accepts at most 3 segments, got {count}")]
    TooManyIndirectSegments { count: usize },
}

/// What the parser was looking for when it failed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParseExpected {
    /// A specific literal token, e.g. the `//` after `hg+https:`.
    Tag(&'static str),
}

impl fmt::Display for ParseExpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(t) => write!(f, "tag `{t}`"),
        }
    }
}

/// Categorised reason an `Unsupported` URI was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnsupportedReason {
    /// A query parameter is not supported by the flakeref type.
    Param { name: String },
    /// A field is recognised but only valid on a different set of types.
    Field {
        field: String,
        only_supported_by: String,
    },
    /// The URI scheme/type identifier is not known.
    UriType { ty: String },
    /// The transport layer (the part after `+`) is not known.
    TransportLayer { ty: String },
    /// A required parameter for the type is missing.
    MissingParameter { ty: String, parameter: String },
    /// The scheme does not accept a URL authority (`//host`). Nix rejects
    /// the same shape; `path://host/...` is malformed even though it
    /// superficially looks like a URL.
    Authority { scheme: &'static str },
    /// A `#fragment` in a [`Context`](crate::Context) that does not take
    /// one. Only [`Context::Installable`](crate::Context::Installable)
    /// allows a fragment.
    Fragment { fragment: String },
    /// A relative path in a [`Context`](crate::Context) that only takes
    /// absolute paths ([`Context::Standalone`](crate::Context::Standalone)).
    /// Any input that is neither a flake id nor a valid URL is read as a
    /// path, so this is also what an unparseable URL turns into there.
    RelativePath { path: String },
    /// An SCP-style git location (`git@github.com:owner/repo`), either
    /// bare or after `git+ssh://`. It is not a flake reference URL. The
    /// equivalent is `git+ssh://git@github.com/owner/repo`.
    ScpStyleUrl { url: String },
}

impl fmt::Display for UnsupportedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Param { name } => {
                write!(
                    f,
                    "the parameter `{name}` is not supported by the flakeref type"
                )
            }
            Self::Field {
                field,
                only_supported_by,
            } => write!(f, "field `{field}` only supported by `{only_supported_by}`"),
            Self::UriType { ty } => write!(f, "unknown URI type `{ty}`"),
            Self::TransportLayer { ty } => write!(f, "unknown transport layer `{ty}`"),
            Self::MissingParameter { ty, parameter } => write!(
                f,
                "FlakeRef type `{ty}` is missing required parameter `{parameter}`"
            ),
            Self::Authority { scheme } => {
                write!(f, "the `{scheme}:` scheme does not accept a URL authority")
            }
            Self::Fragment { fragment } => write!(
                f,
                "unexpected fragment `{fragment}`, only installables take a `#fragment`"
            ),
            Self::RelativePath { path } => write!(
                f,
                "`{path}` is not an absolute path or a valid flake reference URL"
            ),
            Self::ScpStyleUrl { url } => write!(
                f,
                "`{url}` is an SCP-style git location, write it as `git+ssh://<user>@<host>/<path>`"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_position_reports_missing_authority() {
        // `hg+https:` must be followed by `//`.
        let err = crate::FlakeRef::parse("hg+https:x", crate::Context::Standalone).unwrap_err();
        match err {
            NixUriError::Parse { position, expected } => {
                assert_eq!(position, 9, "expected offset 9, got {position}");
                assert_eq!(expected, ParseExpected::Tag("//"));
            }
            other => panic!("expected NixUriError::Parse, got {other:?}"),
        }
    }
}
