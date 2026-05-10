use std::fmt;

use thiserror::Error;
use winnow::Parser;
use winnow::error::{ContextError, ErrMode, ParseError, StrContext, StrContextValue};
use winnow::stream::{Offset, Stream};
use winnow::token::literal;

pub type NixUriResult<T> = Result<T, NixUriError>;

/// Failures the parser may produce.
///
/// Variants are stable categories: callers can match `Parse` for a syntactic
/// failure with a byte position, `Unsupported` for a recognised input that
/// asks for something the library does not implement, `InvalidUrl` for input
/// that fails the high-level URL shape, and `InvalidValue` for a field that
/// passed parsing but failed validation. Structural failures that involve a
/// relationship between fields (rather than a single value's literal shape)
/// surface as named variants such as `FieldConflict`, `MissingScheme`, or
/// `TooManyIndirectSegments`. `ServoUrl` wraps the upstream
/// `url::ParseError` for tarball- and HTTP-style URLs.
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
    /// A bare (no-scheme) input had more than one path segment, so it
    /// cannot be classified as an indirect flake id and the parser cannot
    /// guess which forge scheme (`github:` / `gitlab:` / `sourcehut:`)
    /// the user intended. Mirrors upstream's rejection of bare
    /// `owner/repo` shorthand.
    #[error("input `{input}` has no scheme; bare two-segment shorthand is not supported")]
    MissingScheme { input: String },
    /// A `flake:` indirect URI exceeded Nix's three-segment cap
    /// (`id[/ref[/rev]]`). `count` is the number of `/`-separated
    /// segments in the rejected tail, including the id segment.
    #[error("indirect form accepts at most 3 segments, got {count}")]
    TooManyIndirectSegments { count: usize },
    /// Wraps `url::ParseError` for tarball- and HTTP-style URLs.
    #[error("URL parsing error: {0}")]
    ServoUrl(#[from] url::ParseError),
}

/// What the parser was looking for when it failed.
///
/// `Other` is retained in the public enum so downstream additions can land
/// without churning every caller's match arms, but no internal call site
/// constructs it: every reachable parser fall-through routes into one of
/// the named variants below. A new winnow `StrContextValue` variant
/// added upstream surfaces as `Unknown` rather than silently widening
/// `Other`'s vocabulary at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParseExpected {
    /// A specific literal token, e.g. `github:`.
    Tag(&'static str),
    /// A specific character.
    Char(char),
    /// End of input.
    Eof,
    /// An alphabetic character.
    Alpha,
    /// A decimal digit.
    Digit,
    /// A hexadecimal digit.
    HexDigit,
    /// An alphanumeric character.
    AlphaNumeric,
    /// A space character.
    Space,
    /// One or more whitespace characters.
    Multispace,
    /// A textual description from a winnow `StrContext::Expected`
    /// (`StrContextValue::Description`) frame that the converter did not
    /// fold into one of the typed character-class variants above.
    Description(&'static str),
    /// A `StrContext::Label` frame that survived to the boundary because
    /// no `Expected(...)` frame was attached. The parser-internal
    /// label name is forwarded verbatim.
    Label(&'static str),
    /// The parser ran a multi-branch alternative and none of the branches
    /// pushed a more specific frame.
    Alternatives,
    /// A `StrContextValue` variant from a future winnow upgrade that this
    /// crate has not yet learned to discriminate. Surfacing it as a typed
    /// variant (rather than a runtime string) makes the upgrade an audited
    /// event rather than silent vocabulary drift.
    Unknown,
    /// A free-form description. Reserved for ad-hoc additions; no internal
    /// call site constructs this variant today.
    Other(String),
}

impl fmt::Display for ParseExpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(t) => write!(f, "tag `{t}`"),
            Self::Char(c) => write!(f, "char `{c}`"),
            Self::Eof => f.write_str("end of input"),
            Self::Alpha => f.write_str("an alphabetic character"),
            Self::Digit => f.write_str("a digit"),
            Self::HexDigit => f.write_str("a hex digit"),
            Self::AlphaNumeric => f.write_str("an alphanumeric character"),
            Self::Space => f.write_str("a space"),
            Self::Multispace => f.write_str("whitespace"),
            Self::Description(d) => f.write_str(d),
            Self::Label(s) => write!(f, "label `{s}`"),
            Self::Alternatives => f.write_str("one of several alternatives"),
            Self::Unknown => f.write_str("an unrecognised parser context"),
            Self::Other(s) => f.write_str(s),
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
        }
    }
}

/// A literal-token parser that attaches an `Expected(StringLiteral(...))`
/// context so failures surface as `ParseExpected::Tag(literal)`.
///
/// Carries the crate's failure-shape contract: the `Expected` context is
/// what the boundary converter folds into the `ParseExpected::Tag` variant.
pub(crate) fn tag<'i>(
    expected: &'static str,
) -> impl Parser<&'i str, &'i str, ErrMode<ContextError>> {
    literal(expected).context(StrContext::Expected(StrContextValue::StringLiteral(
        expected,
    )))
}

/// Convert a winnow `ParseError` (the top-level `Parser::parse` failure type)
/// into the public `NixUriError::Parse`.
///
/// `original` is the input slice the parser entry point received. The
/// `ParseError` carries a checkpoint-based offset relative to its own input;
/// the converter shifts that into the `original`'s coordinate space so
/// callers always see byte offsets from the URI's first character.
#[allow(dead_code)]
pub(crate) fn parse_error_from_winnow(
    original: &str,
    pe: &ParseError<&str, ContextError>,
) -> NixUriError {
    let position = offset_within(original, pe.input()) + pe.offset();
    NixUriError::Parse {
        position,
        expected: parse_expected_from_context(pe.inner()),
    }
}

/// Run a parser that does NOT consume to end-of-input (the `parse_peek`
/// shape) and surface failures through the public `Parse` variant.
///
/// `original` is the parser entry-point's input slice; `input` is the
/// (possibly partial) slice this call should parse from. On error the byte
/// position is reported in `original`'s coordinate space.
pub(crate) fn run_partial<'i, P, O>(
    original: &'i str,
    input: &'i str,
    mut parser: P,
) -> Result<(&'i str, O), NixUriError>
where
    P: Parser<&'i str, O, ErrMode<ContextError>>,
{
    let mut current = input;
    let start = current.checkpoint();
    match parser.parse_next(&mut current) {
        Ok(o) => Ok((current, o)),
        Err(err_mode) => {
            let inner = match err_mode {
                ErrMode::Backtrack(e) | ErrMode::Cut(e) => e,
                ErrMode::Incomplete(_) => {
                    unreachable!("complete parsers do not return Incomplete")
                }
            };
            let local_offset = current.offset_from(&start);
            let position = offset_within(original, input) + local_offset;
            Err(NixUriError::Parse {
                position,
                expected: parse_expected_from_context(&inner),
            })
        }
    }
}

/// Walk a `ContextError`'s context list front-to-back (innermost first by
/// winnow's push order) and pick the first `Expected(...)` frame, falling
/// back to the first `Label(...)` frame.
pub(crate) fn parse_expected_from_context(err: &ContextError) -> ParseExpected {
    for ctx in err.context() {
        if let StrContext::Expected(value) = ctx {
            return match value {
                StrContextValue::StringLiteral(s) => ParseExpected::Tag(s),
                StrContextValue::CharLiteral(c) => ParseExpected::Char(*c),
                StrContextValue::Description(d) => description_to_expected(d),
                _ => ParseExpected::Unknown,
            };
        }
    }
    for ctx in err.context() {
        if let StrContext::Label(s) = ctx {
            return ParseExpected::Label(s);
        }
    }
    ParseExpected::Alternatives
}

/// The finite vocabulary of `Description` strings the parser emits today.
/// Anything not in this table folds to `ParseExpected::Description(d)`,
/// which keeps the converter resilient to ad-hoc descriptions added by
/// future parsers without breaking the typed-variant mapping.
fn description_to_expected(d: &'static str) -> ParseExpected {
    match d {
        "end of input" => ParseExpected::Eof,
        "an alphabetic character" => ParseExpected::Alpha,
        "a digit" => ParseExpected::Digit,
        "a hex digit" => ParseExpected::HexDigit,
        "an alphanumeric character" => ParseExpected::AlphaNumeric,
        "a space" => ParseExpected::Space,
        "whitespace" => ParseExpected::Multispace,
        other => ParseExpected::Description(other),
    }
}

/// Returns the byte offset of `slice` within `original`, or 0 if `slice`
/// is not a sub-slice of `original`.
///
/// Contract: `slice` must be a sub-slice of `original`. Every input the
/// parser drives is borrowed from the entry point's input; if a future
/// caller hands an unrelated slice this returns 0 in release and the
/// `debug_assert!` catches the regression in dev builds.
fn offset_within(original: &str, slice: &str) -> usize {
    let orig_start = original.as_ptr() as usize;
    let orig_end = orig_start.saturating_add(original.len());
    let s_start = slice.as_ptr() as usize;
    debug_assert!(
        s_start >= orig_start && s_start <= orig_end,
        "offset_within: slice is not a sub-slice of original",
    );
    if s_start >= orig_start && s_start <= orig_end {
        s_start - orig_start
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_iter_returns_innermost_first() {
        // Pins winnow's `ContextError::context()` iteration order. The
        // converter scans front-to-back and returns the first
        // `Expected(...)`; if winnow ever flips this, the public
        // `ParseExpected` value would silently shift from the inner
        // discriminator to whatever outer label the parse-stack pushed last.
        let r: Result<&str, ParseError<&str, ContextError>> =
            literal::<_, &str, ErrMode<ContextError>>("x")
                .context(StrContext::Expected(StrContextValue::StringLiteral("x")))
                .context(StrContext::Label("outer"))
                .parse("abc");
        let inner = r.unwrap_err().into_inner();
        let labels: Vec<_> = inner.context().collect();
        assert_eq!(labels.len(), 2);
        match labels[0] {
            StrContext::Expected(StrContextValue::StringLiteral(s)) => assert_eq!(*s, "x"),
            other => panic!("expected innermost StringLiteral(\"x\"), got {other:?}"),
        }
        match labels[1] {
            StrContext::Label(s) => assert_eq!(*s, "outer"),
            other => panic!("expected outermost Label(\"outer\"), got {other:?}"),
        }
    }

    #[test]
    fn parse_position_reports_owner_repo_separator() {
        // `parse_owner_repo_ref` consumes `n` as the owner and then
        // demands `/`; the failure must be reported at byte 8 (the
        // `/`'s expected column in the original input), not at the
        // `:` (byte 6) where the GitForge arm was selected, nor at
        // byte 0. Pins both the offset translation through
        // `run_partial` and the `Char('/')` projection of the
        // `StrContextValue::CharLiteral` context.
        let err = "github:n".parse::<crate::FlakeRef>().unwrap_err();
        match err {
            NixUriError::Parse { position, expected } => {
                assert_eq!(position, 8, "expected offset 8, got {position}");
                assert_eq!(
                    expected,
                    ParseExpected::Char('/'),
                    "expected Char('/'), got {expected:?}",
                );
            }
            other => panic!("expected NixUriError::Parse, got {other:?}"),
        }
    }

    #[test]
    fn description_table_round_trips() {
        // The static lookup table is the single source of truth for which
        // `Description` strings fold into a typed variant. This pins the
        // round-trip: every description known today maps to the expected
        // typed variant, and an unknown description falls through to Other.
        assert_eq!(description_to_expected("end of input"), ParseExpected::Eof);
        assert_eq!(
            description_to_expected("an alphabetic character"),
            ParseExpected::Alpha
        );
        assert_eq!(description_to_expected("a digit"), ParseExpected::Digit);
        assert_eq!(
            description_to_expected("a hex digit"),
            ParseExpected::HexDigit
        );
        assert_eq!(
            description_to_expected("an alphanumeric character"),
            ParseExpected::AlphaNumeric,
        );
        assert_eq!(description_to_expected("a space"), ParseExpected::Space);
        assert_eq!(
            description_to_expected("whitespace"),
            ParseExpected::Multispace
        );
        assert_eq!(
            description_to_expected("not in the table"),
            ParseExpected::Description("not in the table"),
        );
    }

    #[test]
    fn parse_expected_does_not_collapse_to_other() {
        // The four internal `parse_expected_from_context` /
        // `description_to_expected` fall-throughs must each route into a
        // typed variant. `Other(_)` stays in the public enum for ad-hoc
        // future additions but is unreachable from production code,
        // so downstream pattern-matchers can discriminate the failure
        // category instead of string-comparing implementation-detail wording.
        for input in [
            "github:!",
            "github:o/r?ref=invalid ref",
            "garbage::scheme",
            "github:",
            "github:nixos/",
        ] {
            let err = input.parse::<crate::FlakeRef>().unwrap_err();
            if let NixUriError::Parse { expected, .. } = err {
                assert!(
                    !matches!(expected, ParseExpected::Other(_)),
                    "input {input:?} produced ParseExpected::Other({expected:?})",
                );
            }
        }
    }
}
