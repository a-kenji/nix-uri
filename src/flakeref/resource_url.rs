use std::fmt::Display;

use serde::{Deserialize, Serialize};
use winnow::{
    ModalResult, Parser,
    combinator::{alt, opt},
    error::StrContext,
    token::take_till,
};

use crate::{error::tag, parser::parse_sep};

use super::{RefLocation, TransportLayer};

/// A resource-style flake reference (`git+https://...`, `hg+ssh://...`,
/// `file+https://...`, `tarball+https://...`).
///
/// Like [`super::GitForge`] and [`super::FlakeRefType::Indirect`], `ref_` and
/// `rev` are typed slots fed from `?ref=` / `?rev=` query parameters at parse
/// time. `ref_location` records where a present value would be rendered on
/// `Display` so round-trips preserve the form. Resource only supports the
/// query-parameter form (not the path-component form `GitForge`/`Indirect`
/// have); `RefLocation::PathComponent` is the slot's default and the parser
/// flips to `QueryParameter` whenever it routes a query-string value here.
///
/// `#[non_exhaustive]` reserves room for future fields without breaking
/// downstream match arms; in-crate construction with struct-literal syntax
/// stays allowed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceUrl {
    pub res_type: ResourceType,
    pub location: String,
    pub transport_type: Option<TransportLayer>,
    pub ref_: Option<String>,
    pub rev: Option<String>,
    pub ref_location: RefLocation,
}

impl ResourceUrl {
    /// Construct a `ResourceUrl` with no ref or rev set; the ref/rev slots
    /// land via the parser's query-string side-channel
    /// (`crate::parser::apply_param_ref_rev`) once parsing completes.
    pub fn new(
        res_type: ResourceType,
        location: String,
        transport_type: Option<TransportLayer>,
    ) -> Self {
        Self {
            res_type,
            location,
            transport_type,
            ref_: None,
            rev: None,
            ref_location: RefLocation::PathComponent,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn parse(input: &mut &str) -> ModalResult<Self> {
        let res_type = ResourceType::parse(input)?;
        let transport_type = opt(TransportLayer::plus_parse).parse_next(input)?;
        let _ = parse_sep(input)?;
        let location = take_till(0.., |c| c == '#' || c == '?')
            .context(StrContext::Label("url location"))
            .parse_next(input)?;
        Ok(Self::new(res_type, location.to_string(), transport_type))
    }
}

/// The resource flavour of a [`ResourceUrl`]: which canonical Nix scheme
/// (`git+`, `hg+`, `file+`, `tarball+`) the URL belongs to. Used to pick
/// the leading scheme token on `Display`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceType {
    Git,
    Mercurial,
    File,
    Tarball,
}

impl ResourceType {
    #[allow(dead_code)]
    pub(crate) fn parse(input: &mut &str) -> ModalResult<Self> {
        alt((
            tag("git").value(Self::Git),
            tag("hg").value(Self::Mercurial),
            tag("file").value(Self::File),
            tag("tarball").value(Self::Tarball),
        ))
        .context(StrContext::Label("resource selection"))
        .parse_next(input)
    }
}

impl Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let out_str = match self {
            Self::Git => "git",
            Self::Mercurial => "hg",
            Self::File => "file",
            Self::Tarball => "tarball",
        };
        write!(f, "{}", out_str)
    }
}

#[cfg(test)]
mod res_url {
    use cool_asserts::assert_matches;

    use super::*;

    #[test]
    fn git() {
        let url = "gitfoobar";
        let (rest, parsed) = ResourceType::parse.parse_peek(url).unwrap();
        let expected = ResourceType::Git;
        assert_eq!(expected, parsed);
        assert_eq!("foobar", rest);
    }

    #[test]
    fn unknown_resource_scheme_routes_to_uri_type_unsupported() {
        use crate::{NixUriError, error::UnsupportedReason, parser::parse_nix_uri};

        assert_matches!(
            parse_nix_uri("gat://x"),
            Err(NixUriError::Unsupported(UnsupportedReason::UriType { ty }))
                => assert_eq!(ty, "gat")
        );
    }
}
