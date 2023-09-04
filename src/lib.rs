//! Convenience functionality for working with nix `flake.nix` references
//! (flakerefs)
//! Provides types for the generic attribute set representation, but does not parse it:
//!
//! ``no_run
//!    {
//!      type = "github";
//!      owner = "NixOS";
//!      repo = "nixpkgs";
//!    }
//! ``
//!
//! The uri syntax representation is parsed by this library:
//! example `github:a-kenji/nala`
// use nom::complete::tag;
use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    character::complete::{alphanumeric0, anychar},
    combinator::{opt, rest},
    multi::many_m_n,
    sequence::preceded,
    IResult,
};
use serde::{Deserialize, Serialize};

/// The General Flake Ref Schema
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
struct FlakeRef {
    r#type: FlakeRefType,
    owner: Option<String>,
    repo: Option<String>,
    flake: Option<bool>,
    rev_or_ref: Option<String>,
    r#ref: Option<String>,
    attrs: FlakeRefAttributes,
}

fn parse_owner_repo_ref(input: &str) -> IResult<&str, Vec<&str>> {
    use nom::sequence::separated_pair;
    let (input, owner_or_ref) = many_m_n(
        0,
        3,
        separated_pair(
            take_until("/"),
            tag("/"),
            alt((take_until("/"), take_until("?"), rest)),
        ),
    )(input)?;

    let owner_and_rev_or_ref: Vec<&str> = owner_or_ref
        .clone()
        .into_iter()
        .flat_map(|(x, y)| vec![x, y])
        .filter(|s| !s.is_empty())
        .collect();
    Ok((input, owner_and_rev_or_ref))
}

/// Take all that is behind the "?" tag
/// Return everything prior as not parsed
fn parse_params(input: &str) -> IResult<&str, Option<FlakeRefAttributes>> {
    use nom::sequence::separated_pair;

    // This is the inverse of the general control flow
    let (input, maybe_flake_type) = opt(take_until("?"))(input)?;

    println!("Input: {input}");
    println!("flake_type: {maybe_flake_type:?}");

    if let Some(flake_type) = maybe_flake_type {
        // discard leading "?"
        let (input, _) = anychar(input)?;
        let (input, param_values) = many_m_n(
            0,
            11,
            separated_pair(take_until("="), tag("="), alt((take_until("&"), rest))),
        )(input)?;
        println!("Not parsed yet: {}", input);
        println!("Params : {:?}", param_values);
        println!("Not parsed yet: {}", input);

        let mut attrs = FlakeRefAttributes::default();
        for (param, value) in param_values {
            // param can start with "&"
            // TODO: actual error handling instead of unwrapping
            match param.parse().unwrap() {
                FlakeRefParam::Dir => {
                    attrs.dir(Some(value.into()));
                }
                FlakeRefParam::NarHash => {
                    attrs.nar_hash(Some(value.into()));
                }
                FlakeRefParam::Host => todo!(),
            }
        }
        Ok((flake_type, Some(attrs)))
    } else {
        Ok((input, None))
    }
}

fn parse_nix_uri(input: &str) -> IResult<&str, FlakeRef> {
    let (input, attrs) = parse_params(input)?;
    println!("Input: {input}");
    println!("Attrs: {attrs:?}");
    let mut flake_ref = FlakeRef::default();
    let (input, flake_ref_type) = FlakeRefType::parse_type(input)?;
    flake_ref.r#type(flake_ref_type);
    if let Some(attrs) = attrs {
        flake_ref.attrs(attrs);
    }

    Ok((input, flake_ref))
}

impl FlakeRef {
    fn r#type(&mut self, r#type: FlakeRefType) -> &mut Self {
        self.r#type = r#type;
        self
    }
    fn owner(&mut self, owner: Option<String>) -> &mut Self {
        self.owner = owner;
        self
    }
    fn repo(&mut self, repo: Option<String>) -> &mut Self {
        self.repo = repo;
        self
    }

    fn rev_or_ref(&mut self, rev_or_ref: Option<String>) -> &mut Self {
        self.rev_or_ref = rev_or_ref;
        self
    }

    fn attrs(&mut self, attrs: FlakeRefAttributes) -> &mut Self {
        self.attrs = attrs;
        self
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
pub struct FlakeRefAttributes {
    dir: Option<String>,
    #[serde(rename = "narHash")]
    nar_hash: Option<String>,
    rev: Option<String>,
    r#ref: Option<String>,
    // Only available to certain types
    host: Option<String>,
    // Not available to user
    #[serde(rename = "revCount")]
    rev_count: Option<String>,
    // Not available to user
    #[serde(rename = "lastModified")]
    last_modified: Option<String>,
}

impl FlakeRefAttributes {
    fn dir(&mut self, dir: Option<String>) -> &mut Self {
        self.dir = dir;
        self
    }

    fn nar_hash(&mut self, nar_hash: Option<String>) -> &mut Self {
        self.nar_hash = nar_hash;
        self
    }
}

pub enum FlakeRefParam {
    Dir,
    NarHash,
    Host,
}

impl std::str::FromStr for FlakeRefParam {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use FlakeRefParam::*;
        match s {
            "dir" | "&dir" => Ok(Dir),
            "nar_hash" | "&nar_hash" => Ok(NarHash),
            "host" | "&host" => Ok(Host),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum FlakeRefType {
    // In URL form, the schema must be file+http://, file+https:// or file+file://. If the extension doesn’t correspond to a known archive format (as defined by the tarball fetcher), then the file+ prefix can be dropped.
    File(String),
    /// Git repositories. The location of the repository is specified by the attribute `url`.
    Git,
    GitHub {
        owner: String,
        repo: String,
        ref_or_rev: Option<String>,
    },
    Indirect,
    Mercurial,
    /// Path must be a directory in the filesystem containing a `flake.nix`.
    /// Path must be an absolute path.
    Path {
        path: String,
    },
    Sourcehut,
    Tarball,
    #[default]
    None,
}

impl FlakeRefType {
    /// Parse type specific information, returns the [`FlakeRefType`]
    /// and the unparsed input
    pub fn parse_type(input: &str) -> IResult<&str, FlakeRefType> {
        use nom::sequence::separated_pair;
        let (_, (flake_ref_type, input)) = separated_pair(alphanumeric0, tag(":"), rest)(input)?;
        match flake_ref_type {
            "github" => {
                let (input, owner_and_repo_or_ref) = parse_owner_repo_ref(input)?;
                let flake_ref_type = FlakeRefType::GitHub {
                    owner: owner_and_repo_or_ref[0].into(),
                    repo: owner_and_repo_or_ref[1].into(),
                    ref_or_rev: owner_and_repo_or_ref
                        .get(2)
                        .cloned()
                        .map(|s| s.clone().to_string()),
                };
                Ok((input, flake_ref_type))
            }
            "path" => {
                let flake_ref_type = FlakeRefType::Path { path: input.into() };
                Ok(("", flake_ref_type))
            }

            _ => todo!("Error"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_uri_nom() {
        let uri = "github:zellij-org/zellij";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_uri_nom_params() {
        let uri = "github:zellij-org/zellij";
        let flake_attrs = None;
        let parsed = parse_params(uri).unwrap();
        assert_eq!(("github:zellij-org/zellij", flake_attrs), parsed);
    }
    #[test]
    fn parse_simple_uri_attr_nom_params() {
        let uri = "github:zellij-org/zellij?dir=assets";
        let mut flake_attrs = FlakeRefAttributes::default();
        flake_attrs.dir(Some("assets".into()));
        let parsed = parse_params(uri).unwrap();
        assert_eq!(("github:zellij-org/zellij", Some(flake_attrs)), parsed);
    }

    #[test]
    fn parse_simple_uri_ref_or_rev_nom() {
        let uri = "github:zellij-org/zellij/main";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: Some("main".into()),
            })
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_uri_ref_or_rev_attr_nom() {
        let uri = "github:zellij-org/zellij/main?dir=assets";
        let mut attrs = FlakeRefAttributes::default();
        attrs.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: Some("main".into()),
            })
            .attrs(attrs)
            .clone();

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_uri_attr_nom() {
        let uri = "github:zellij-org/zellij?dir=assets";
        let mut attrs = FlakeRefAttributes::default();
        attrs.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .attrs(attrs)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_uri_attr_nom_alt() {
        let uri = "github:zellij-org/zellij/?dir=assets";
        let mut attrs = FlakeRefAttributes::default();
        attrs.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .attrs(attrs)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_uri_attrs_nom_alt() {
        let uri = "github:zellij-org/zellij/?dir=assets&nar_hash=fakeHash256";
        let mut attrs = FlakeRefAttributes::default();
        attrs.dir(Some("assets".into()));
        attrs.nar_hash(Some("fakeHash256".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .attrs(attrs)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_path_nom() {
        let uri = "path:/home/kenji/.config/dotfiles/";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/.config/dotfiles/".into(),
            })
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
    #[test]
    fn parse_simple_path_attrs_nom() {
        let uri = "path:/home/kenji/.config/dotfiles/?dir=assets";
        let mut attrs = FlakeRefAttributes::default();
        attrs.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/.config/dotfiles/".into(),
            })
            .attrs(attrs)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(("", flake_ref), parsed);
    }
}
