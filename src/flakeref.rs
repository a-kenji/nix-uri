use std::fmt::Display;

use serde::{Deserialize, Serialize};
use winnow::{
    combinator::{opt, trace},
    error::{StrContext, StrContextValue},
    PResult, Parser,
};

use crate::{error::NixUriError, parser::parse_nix_uri};

mod fr_type;
pub use fr_type::FlakeRefType;
mod location_params;
pub use location_params::{LocationParamKeys, LocationParameters};
mod transport_layer;
pub use transport_layer::TransportLayer;
mod forge;
pub use forge::{GitForge, GitForgePlatform};
mod resource_url;

/// The General Flake Ref Schema
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
pub struct FlakeRef {
    pub r#type: FlakeRefType,
    flake: Option<bool>,
    pub params: LocationParameters,
}

impl FlakeRef {
    pub fn new(r#type: FlakeRefType) -> Self {
        Self {
            r#type,
            ..Self::default()
        }
    }

    pub fn from<S>(input: S) -> Result<Self, NixUriError>
    where
        S: AsRef<str>,
    {
        TryInto::<Self>::try_into(input.as_ref())
    }

    pub fn r#type(&mut self, r#type: FlakeRefType) -> &mut Self {
        self.r#type = r#type;
        self
    }
    pub fn id(&self) -> Option<String> {
        self.r#type.get_id()
    }

    pub fn params(&mut self, params: LocationParameters) -> &mut Self {
        self.params = params;
        self
    }
    pub fn parse(input: &mut &str) -> PResult<Self> {
        let r#type = trace("getting type", FlakeRefType::parse)
            .context(StrContext::Label("FlakeRef"))
            .parse_next(input)?;
        let params = opt(trace(
            "getting params",
            LocationParameters::parse_preceded.context(StrContext::Label("LocationParams")),
        ))
        .parse_next(input)?;
        Ok(Self {
            r#type,
            flake: None,
            params: params.unwrap_or_default(),
        })
    }
}

impl Display for FlakeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: convert into Option
        let params = self.params.to_string();
        if params.is_empty() {
            write!(f, "{}", self.r#type)
        } else {
            write!(f, "{}?{params}", self.r#type)
        }
    }
}

impl TryFrom<&str> for FlakeRef {
    type Error = NixUriError;

    fn try_from(mut value: &str) -> Result<Self, Self::Error> {
        parse_nix_uri(&mut value)
    }
}

impl std::str::FromStr for FlakeRef {
    type Err = NixUriError;

    fn from_str(mut s: &str) -> Result<Self, Self::Err> {
        use crate::parser::parse_nix_uri;
        parse_nix_uri(&mut s)
    }
}

#[cfg(test)]
mod inc_parse {

    use resource_url::{ResourceType, ResourceUrl};

    use super::*;
    #[test]
    fn full_github() {
        let mut uri = "github:owner/repo/rev?dir=foo#fizz.buzz";
        let mut expected = FlakeRef::default();
        expected.r#type(FlakeRefType::GitForge(GitForge {
            platform: GitForgePlatform::GitHub,
            owner: "owner".into(),
            repo: "repo".into(),
            ref_or_rev: Some("rev".to_string()),
        }));
        let mut exp_params = LocationParameters::default();
        exp_params.dir(Some("foo".to_string()));
        expected.params = exp_params;

        let parse_out = FlakeRef::parse(&mut uri).unwrap();

        // TODO: when attrs are implemented, this should assert `""`
        assert_eq!("#fizz.buzz", uri);
        assert_eq!(expected, parse_out);
    }
    #[test]
    fn full_path() {
        let mut uri = "file:///phantom/root/path?dir=foo#fizz.buzz";
        let mut expected = FlakeRef::default();
        expected.r#type(FlakeRefType::Resource(ResourceUrl {
            res_type: ResourceType::File,
            location: "/phantom/root/path".to_string(),
            transport_type: None,
        }));
        let mut exp_params = LocationParameters::default();
        exp_params.dir(Some("foo".to_string()));
        expected.params = exp_params;

        let parse_out = FlakeRef::parse(&mut uri).unwrap();
        // TODO: when attrs are implemented, this should assert `""`
        assert_eq!("#fizz.buzz", uri);
        assert_eq!(expected, parse_out);
    }
}

#[cfg(test)]
mod tests {

    use resource_url::{ResourceType, ResourceUrl};

    use super::*;
    use crate::NixUriResult;

    #[test]
    fn parse_simple_uri() {
        let mut uri = "github:nixos/nixpkgs";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "nixos".into(),
                repo: "nixpkgs".into(),
                ref_or_rev: None,
            }))
            .clone();

        let parsed: FlakeRef = uri.try_into().unwrap();
        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, parsed);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_simple_uri_slashed() {
        let mut uri = "github:nixos/nixpkgs/";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "nixos".into(),
                repo: "nixpkgs".into(),
                ref_or_rev: None,
            }))
            .clone();

        let parsed: FlakeRef = uri.try_into().unwrap();
        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, parsed);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_simple_uri_ref() {
        let mut uri = "github:zellij-org/zellij?ref=main";
        let mut flake_attrs = LocationParameters::default();
        flake_attrs.r#ref(Some("main".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            }))
            .params(flake_attrs)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }

    #[test]
    fn parse_simple_uri_ref_slashed() {
        let mut uri = "github:zellij-org/zellij/?ref=main";
        let mut flake_attrs = LocationParameters::default();
        flake_attrs.r#ref(Some("main".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            }))
            .params(flake_attrs)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }

    #[test]
    fn parse_simple_uri_rev() {
        let mut uri = "github:zellij-org/zellij?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298";
        let mut flake_attrs = LocationParameters::default();
        flake_attrs.rev(Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            }))
            .params(flake_attrs)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }

    #[test]
    fn parse_simple_uri_ref_or_rev() {
        let mut uri = "github:zellij-org/zellij/main";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: Some("main".into()),
            }))
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }

    #[test]
    fn parse_simple_uri_ref_or_rev_attr() {
        let mut uri = "github:zellij-org/zellij/main?dir=assets";
        let mut params = LocationParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: Some("main".into()),
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }

    #[test]
    fn parse_simple_uri_attr() {
        let mut uri = "github:zellij-org/zellij?dir=assets";
        let mut params = LocationParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }
    #[test]
    fn parse_simple_uri_attr_nom_alt() {
        let mut uri = "github:zellij-org/zellij/?dir=assets";
        let mut params = LocationParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }
    #[test]
    fn parse_simple_uri_params_nom_alt() {
        let mut uri = "github:zellij-org/zellij/?dir=assets&nar_hash=fakeHash256";
        let mut params = LocationParameters::default();
        params.dir(Some("assets".into()));
        params.nar_hash(Some("fakeHash256".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }

    #[test]
    fn parse_simple_path_nom() {
        let mut uri = "path:/home/kenji/.config/dotfiles/";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/.config/dotfiles/".into(),
            })
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed, "{}", uri);
    }

    #[test]
    fn parse_simple_path_params_nom() {
        let mut uri = "path:/home/kenji/.config/dotfiles/?dir=assets";
        let mut params = LocationParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/.config/dotfiles/".into(),
            })
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed, "{}", uri);
    }

    #[test]
    fn parse_gitlab_simple() {
        let mut uri = "gitlab:veloren/veloren";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "veloren".into(),
                repo: "veloren".into(),
                ref_or_rev: None,
            }))
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }

    #[test]
    fn parse_gitlab_simple_ref_or_rev() {
        let mut uri = "gitlab:veloren/veloren/master";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "veloren".into(),
                repo: "veloren".into(),
                ref_or_rev: Some("master".into()),
            }))
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }

    #[test]
    fn parse_gitlab_simple_ref_or_rev_alt() {
        let mut uri = "gitlab:veloren/veloren/19742bb9300fb0be9fdc92f30766c95230a8a371";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "veloren".into(),
                repo: "veloren".into(),
                ref_or_rev: Some("19742bb9300fb0be9fdc92f30766c95230a8a371".into()),
            }))
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }

    // TODO: replace / with %2F
    // #[test]
    // fn parse_gitlab_nested_subgroup() {
    //     let uri = "gitlab:veloren%2Fdev/rfcs";
    //     let parsed = parse_nix_uri(uri).unwrap();
    //     let flake_ref = FlakeRef::default()
    //         .r#type(FlakeRefType::GitLab {
    //             owner: "veloren".into(),
    //             repo: "dev".into(),
    //             ref_or_rev: Some("rfcs".to_owned()),
    //         })
    //         .clone();
    //     assert_eq!(("", flake_ref), parsed);
    // }
    //

    #[test]
    fn parse_gitlab_simple_host_param() {
        let mut uri = "gitlab:openldap/openldap?host=git.openldap.org";
        let mut params = LocationParameters::default();
        params.host(Some("git.openldap.org".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "openldap".into(),
                repo: "openldap".into(),
                ref_or_rev: None,
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(flake_ref, nommed);
    }

    #[test]
    fn parse_git_and_https_simple() {
        let mut uri = "git+https://git.somehost.tld/user/path";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "git.somehost.tld/user/path".into(),
                transport_type: Some(TransportLayer::Https),
            }))
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_git_and_https_params() {
        let mut uri = "git+https://git.somehost.tld/user/path?ref=branch&rev=fdc8ef970de2b4634e1b3dca296e1ed918459a9e";
        let mut params = LocationParameters::default();
        params.r#ref(Some("branch".into()));
        params.rev(Some("fdc8ef970de2b4634e1b3dca296e1ed918459a9e".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "git.somehost.tld/user/path".into(),
                transport_type: Some(TransportLayer::Https),
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_git_and_file_params() {
        let mut uri = "git+file:///nix/nixpkgs?ref=upstream/nixpkgs-unstable";
        let mut params = LocationParameters::default();
        params.r#ref(Some("upstream/nixpkgs-unstable".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "/nix/nixpkgs".into(),
                transport_type: Some(TransportLayer::File),
            }))
            .params(params.clone())
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_git_and_file_simple() {
        let mut uri = "git+file:///nix/nixpkgs";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "/nix/nixpkgs".into(),
                transport_type: Some(TransportLayer::File),
            }))
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    // TODO: is this correct?
    // git+file:/home/user/forked-flake?branch=feat/myNewFeature
    fn parse_git_and_file_params_alt() {
        let mut uri = "git+file:///home/user/forked-flake?branch=feat/myNewFeature";
        let mut params = LocationParameters::default();
        params.set_branch(Some("feat/myNewFeature".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "/home/user/forked-flake".into(),
                transport_type: Some(TransportLayer::File),
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_github_simple_tag_non_alphabetic_params() {
        let mut uri = "github:smunix/MyST-Parser?ref=fix.hls-docutils";
        let mut params = LocationParameters::default();
        params.set_ref(Some("fix.hls-docutils".to_owned()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "smunix".into(),
                repo: "MyST-Parser".into(),
                ref_or_rev: None,
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_github_simple_tag() {
        let mut uri = "github:cachix/devenv/v0.5";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "cachix".into(),
                repo: "devenv".into(),
                ref_or_rev: Some("v0.5".into()),
            }))
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_git_and_file_params_alt_branch() {
        let mut uri = "git+file:///home/user/forked-flake?branch=feat/myNewFeature";
        let mut params = LocationParameters::default();
        params.set_branch(Some("feat/myNewFeature".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "/home/user/forked-flake".into(),
                transport_type: Some(TransportLayer::File),
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_gitlab_with_host_params_alt() {
        let mut uri = "gitlab:fpottier/menhir/20201216?host=gitlab.inria.fr";
        let mut params = LocationParameters::default();
        params.set_host(Some("gitlab.inria.fr".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "fpottier".to_owned(),
                repo: "menhir".to_owned(),
                ref_or_rev: Some("20201216".to_owned()),
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_git_and_https_params_submodules() {
        let mut uri = "git+https://www.github.com/ocaml/ocaml-lsp?submodules=1";
        let mut params = LocationParameters::default();
        params.set_submodules(Some("1".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "www.github.com/ocaml/ocaml-lsp".to_owned(),
                transport_type: Some(TransportLayer::Https),
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_marcurial_and_https_simpe_uri() {
        let mut uri = "hg+https://www.github.com/ocaml/ocaml-lsp";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Mercurial,
                location: "www.github.com/ocaml/ocaml-lsp".to_owned(),
                transport_type: Some(TransportLayer::Https),
            }))
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    #[should_panic(
        expected = "called `Result::unwrap()` on an `Err` value: Backtrack(ContextError { context: [Label(\"FlakeRef\")], cause: None })"
    )]
    fn parse_git_and_https_params_submodules_wrong_type() {
        let mut uri = "gt+https://www.github.com/ocaml/ocaml-lsp?submodules=1";
        let mut params = LocationParameters::default();
        params.set_submodules(Some("1".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "www.github.com/ocaml/ocaml-lsp".to_owned(),
                transport_type: Some(TransportLayer::Https),
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    // TODO: https://github.com/a-kenji/nix-uri/issues/157
    #[test]
    fn parse_git_and_file_shallow() {
        let uri = "git+file:///path/to/repo?shallow=1";
        let mut params = LocationParameters::default();
        params.set_shallow(Some("1".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Resource(ResourceUrl {
                res_type: ResourceType::Git,
                location: "/path/to/repo".to_owned(),
                transport_type: Some(TransportLayer::File),
            }))
            .params(params)
            .clone();

        let parsed = uri.try_into();
        // let (rest, nommed) = FlakeRef::parse(uri).unwrap();

        // assert_eq!("", rest);
        assert_eq!(Ok(expected), parsed, "uri: {}", uri);
        // assert_eq!(expected, nommed);
    }

    // TODO: allow them with an optional cli parser
    // #[test]
    // fn parse_simple_path_uri_indirect() {
    //     let uri = "path:../.";
    //     let expected = FlakeRef::default()
    //         .r#type(FlakeRefType::Path {
    //             path: "../.".to_owned(),
    //         })
    //         .clone();
    //     let parsed: FlakeRef = uri.try_into().unwrap();
    //     assert_eq!(expected, parsed);
    // }
    // TODO: allow them with an optional cli parser
    // #[test]
    // fn parse_simple_path_uri_indirect_local() {
    //     let uri = "path:.";
    //     let expected = FlakeRef::default()
    //         .r#type(FlakeRefType::Path {
    //             path: ".".to_owned(),
    //         })
    //         .clone();
    //     let parsed: FlakeRef = uri.try_into().unwrap();
    //     assert_eq!(expected, parsed);
    // }

    #[test]
    fn parse_simple_uri_sourcehut() {
        let mut uri = "sourcehut:~misterio/nix-colors";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: None,
            }))
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_simple_uri_sourcehut_rev() {
        let mut uri = "sourcehut:~misterio/nix-colors/main";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("main".to_owned()),
            }))
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_simple_uri_sourcehut_host_param() {
        let mut uri = "sourcehut:~misterio/nix-colors?host=git.example.org";
        let mut params = LocationParameters::default();
        params.set_host(Some("git.example.org".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: None,
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_simple_uri_sourcehut_ref() {
        let mut uri = "sourcehut:~misterio/nix-colors/182b4b8709b8ffe4e9774a4c5d6877bf6bb9a21c";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("182b4b8709b8ffe4e9774a4c5d6877bf6bb9a21c".to_owned()),
            }))
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_simple_uri_sourcehut_ref_params() {
        let mut uri =
            "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de?host=hg.sr.ht";
        let mut params = LocationParameters::default();
        params.set_host(Some("hg.sr.ht".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
            }))
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn display_simple_sourcehut_uri_ref_or_rev() {
        let expected = "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
            }))
            .to_string();

        assert_eq!(expected, flake_ref);
    }

    #[test]
    fn display_simple_sourcehut_uri_ref_or_rev_host_param() {
        let expected =
            "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de?host=hg.sr.ht";
        let mut params = LocationParameters::default();
        params.set_host(Some("hg.sr.ht".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
            }))
            .params(params)
            .to_string();

        assert_eq!(expected, flake_ref);
    }

    #[test]
    fn display_simple_github_uri_ref() {
        let expected = "github:zellij-org/zellij?ref=main";
        let mut flake_attrs = LocationParameters::default();
        flake_attrs.r#ref(Some("main".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            }))
            .params(flake_attrs)
            .to_string();

        assert_eq!(flake_ref, expected);
    }

    #[test]
    fn display_simple_github_uri_rev() {
        let expected = "github:zellij-org/zellij?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298";
        let mut flake_attrs = LocationParameters::default();
        flake_attrs.rev(Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            }))
            .params(flake_attrs)
            .to_string();

        assert_eq!(flake_ref, expected);
    }

    #[test]
    fn parse_simple_path_uri_indirect_absolute_without_prefix() {
        let mut uri = "/home/kenji/git";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/git".to_owned(),
            })
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    #[test]
    fn parse_simple_path_uri_indirect_absolute_without_prefix_with_params() {
        let mut uri = "/home/kenji/git?dir=dev";
        let mut params = LocationParameters::default();
        params.set_dir(Some("dev".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/git".to_owned(),
            })
            .params(params)
            .clone();

        let nommed = FlakeRef::parse(&mut uri).unwrap();

        assert_eq!("", uri);
        assert_eq!(expected, nommed);
    }

    // TODO: allow them with an optional cli parser

    // #[test]
    // fn parse_simple_path_uri_indirect_local_without_prefix() {
    //     let uri = ".";
    //     let expected = FlakeRef::default()
    //         .r#type(FlakeRefType::Path {
    //             path: ".".to_owned(),
    //         })
    //         .clone();
    //     let parsed: FlakeRef = uri.try_into().unwrap();
    //     assert_eq!(expected, parsed);
    // }

    #[test]
    fn parse_wrong_git_uri_extension_type() {
        let mut uri = "git+(:z";
        let expected = "ctx error: Parsing Error: ContextError { context: [Label(\"FlakeRef\")], cause: None }";
        let parsed: NixUriResult<FlakeRef> = uri.try_into();
        assert_eq!(expected, parsed.unwrap_err().to_string());
        let _e = FlakeRef::parse(&mut uri).unwrap_err();
        // todo: map to good error
        // assert_eq!(expected, e);
    }

    #[test]
    #[ignore = "the nom-parser needs to implement the error now"]
    fn parse_github_missing_parameter() {
        let mut uri = "github:";
        let expected = NixUriError::MissingTypeParameter("github".into(), "owner".into());
        let parsed: NixUriResult<FlakeRef> = uri.try_into();
        assert_eq!(expected, parsed.unwrap_err());
        let _e = FlakeRef::parse(&mut uri).unwrap_err();
        // assert_eq!(expected, e);
    }

    #[test]
    #[ignore = "the nom-parser needs to implement the error now"]
    fn parse_github_missing_parameter_repo() {
        let uri = "github:nixos/";
        let expected = Err(NixUriError::MissingTypeParameter(
            "github".into(),
            "repo".into(),
        ));
        assert_eq!(uri.parse::<FlakeRef>(), expected);
        // let e = FlakeRef::parse(uri).unwrap_err();
        // assert_eq!(expected, e);
    }

    #[test]
    fn parse_github_starts_with_whitespace() {
        let uri = " github:nixos/nixpkgs";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
    }

    #[test]
    fn parse_github_ends_with_whitespace() {
        let uri = "github:nixos/nixpkgs ";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
        // let e = FlakeRef::parse(uri).unwrap_err();
        // assert_eq!(expected, e);
    }

    #[test]
    fn parse_empty_invalid_url() {
        let uri = "";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
        // let e = FlakeRef::parse(uri).unwrap_err();
        // assert_eq!(expected, e);
    }

    #[test]
    fn parse_empty_trim_invalid_url() {
        let uri = "  ";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
        // let e = FlakeRef::parse(uri).unwrap_err();
        // assert_eq!(expected, e);
    }

    #[test]
    fn parse_slash_trim_invalid_url() {
        let uri = "   /   ";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
        // let e = FlakeRef::parse(uri).unwrap_err();
        // assert_eq!(expected, e);
    }

    #[test]
    fn parse_double_trim_invalid_url() {
        let uri = "   :   ";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
        // let e = FlakeRef::parse(uri).unwrap_err();
        // assert_eq!(expected, e);
    }

    // #[test]
    // fn parse_simple_indirect() {
    //     let uri = "nixos/nixpkgs";
    //     let expected = FlakeRef::default()
    //         .r#type(FlakeRefType::Indirect {
    //             id: "nixos/nixpkgs".to_owned(),
    //             ref_or_rev: None,
    //         })
    //         .clone();
    //     let parsed: FlakeRef = uri.try_into().unwrap();
    //     assert_eq!(expected, parsed);
    // }

    // TODO: indirect uris
    // #[test]
    // fn parse_simple_tarball() {
    //     let uri = "https://hackage.haskell.org/package/lsp-test-0.14.0.3/lsp-test-0.14.0.3.tar.gz";
    //     let mut params = LocationParameters::default();
    //     let expected = FlakeRef::default()
    //         .r#type(FlakeRefType::Tarball {
    //             id: "nixpkgs".to_owned(),
    //             ref_or_rev: Some("nixos-23.05".to_owned()),
    //         })
    //         .params(params)
    //         .clone();
    //     let parsed: FlakeRef = uri.try_into().unwrap();
    //     assert_eq!(expected, parsed);
    // }
}
