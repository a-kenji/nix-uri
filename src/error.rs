use std::fmt;

pub type NixUriResult<T> = Result<T, NixUriError>;

/// Failures the parser may produce.
///
/// Variants are stable categories: callers can match `Unsupported` for a recognised input
/// that asks for something the library does not implement.
#[derive(Debug)]
#[non_exhaustive]
pub enum NixUriError {
    /// The input parsed but requests an unsupported operation, parameter,
    /// type, or transport layer.
    Unsupported(UnsupportedReason),
    /// The input is malformed at a higher level than the byte parser:
    /// an illegal path character, a non-absolute path where one was required,
    /// or an otherwise unparseable URL shape.
    InvalidUrl(String),
    /// A field's value did not pass validation. Reserved for failures where
    /// a single field's literal shape is wrong (e.g. a `?rev=` that is not
    /// 40 hex chars). Failures that involve a relationship between two
    /// fields surface as [`Self::FieldConflict`].
    InvalidValue { field: &'static str, reason: String },
    /// Two fields were populated that the grammar treats as mutually exclusive. The
    /// canonical case is a `GitForge` input that supplies both `ref` and `rev`.
    FieldConflict {
        left: &'static str,
        right: &'static str,
    },
    /// A field was given twice, for example a git-forge ref both as a
    /// path segment and as `?ref=` (`github:o/r/main?ref=dev`).
    DuplicateField { field: &'static str },
    /// An indirect reference (a `flake:` URL or a bare flake id) exceeded
    /// the three-segment cap (`id[/ref[/rev]]`). `count` is the number
    /// of `/`-separated segments, including the id segment.
    TooManyIndirectSegments { count: usize },
}

impl fmt::Display for NixUriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(reason) => reason.fmt(f),
            Self::InvalidUrl(url) => write!(f, "not a valid URL: {url}"),
            Self::InvalidValue { field, reason } => {
                write!(f, "invalid value for `{field}`: {reason}")
            }
            Self::FieldConflict { left, right } => {
                write!(f, "`{left}` and `{right}` are mutually exclusive")
            }
            Self::DuplicateField { field } => write!(f, "`{field}` is given more than once"),
            Self::TooManyIndirectSegments { count } => {
                write!(f, "indirect form accepts at most 3 segments, got {count}")
            }
        }
    }
}

impl std::error::Error for NixUriError {}

/// Categorised reason an `Unsupported` URI was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnsupportedReason {
    /// A query parameter or attribute is not supported by the flakeref type.
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
    /// The scheme does not accept a URL authority (`//host`).
    Authority { scheme: &'static str },
    /// A non-empty `#fragment` given to [`FlakeRef::parse`](crate::FlakeRef::parse). A
    /// fragment names an attribute path, see
    /// [`FlakeRef::parse_with_fragment`](crate::FlakeRef::parse_with_fragment).
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
                    "the parameter or attribute `{name}` is not supported by the flakeref type"
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
            Self::Fragment { fragment } => {
                write!(f, "unexpected fragment `{fragment}` in a flake reference")
            }
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
