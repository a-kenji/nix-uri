use std::{fmt::Display, path::Path};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    combinator::{map, opt, rest},
    multi::many_m_n,
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{NixUriError, NixUriResult},
    parser::parse_url_type,
};

mod fr_type;
pub use fr_type::FlakeRefType;
mod fr_params;
pub use fr_params::{FlakeRefParamKeys, FlakeRefParameters};
mod fr_urls;
pub use fr_urls::UrlType;
mod forge;
pub use forge::{GitForge, GitForgePlatform};

/// The General Flake Ref Schema
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
pub struct FlakeRef {
    pub r#type: FlakeRefType,
    flake: Option<bool>,
    pub params: FlakeRefParameters,
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

    pub fn params(&mut self, params: FlakeRefParameters) -> &mut Self {
        self.params = params;
        self
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

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        use crate::parser::parse_nix_uri;
        parse_nix_uri(value)
    }
}

impl std::str::FromStr for FlakeRef {
    type Err = NixUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use crate::parser::parse_nix_uri;
        parse_nix_uri(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{parse_nix_uri, parse_params};

    #[test]
    fn parse_simple_uri() {
        let uri = "github:nixos/nixpkgs";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "nixos".into(),
                repo: "nixpkgs".into(),
                ref_or_rev: None,
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_uri_parsed() {
        let uri = "github:zellij-org/zellij";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .clone();
        let parsed: FlakeRef = uri.parse().unwrap();
        assert_eq!(expected, parsed);
    }

    #[test]
    fn parse_simple_uri_nom() {
        let uri = "github:zellij-org/zellij";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
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
        let mut flake_attrs = FlakeRefParameters::default();
        flake_attrs.dir(Some("assets".into()));
        let parsed = parse_params(uri).unwrap();
        assert_eq!(("github:zellij-org/zellij", Some(flake_attrs)), parsed);
    }
    #[test]
    fn parse_simple_uri_ref() {
        let uri = "github:zellij-org/zellij?ref=main";
        let mut flake_attrs = FlakeRefParameters::default();
        flake_attrs.r#ref(Some("main".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .params(flake_attrs)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_rev() {
        let uri = "github:zellij-org/zellij?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298";
        let mut flake_attrs = FlakeRefParameters::default();
        flake_attrs.rev(Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .params(flake_attrs)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_ref_or_rev_nom() {
        let uri = "github:zellij-org/zellij/main";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: Some("main".into()),
            })
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_ref_or_rev_attr_nom() {
        let uri = "github:zellij-org/zellij/main?dir=assets";
        let mut params = FlakeRefParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: Some("main".into()),
            })
            .params(params)
            .clone();

        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_attr_nom() {
        let uri = "github:zellij-org/zellij?dir=assets";
        let mut params = FlakeRefParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_attr_nom_alt() {
        let uri = "github:zellij-org/zellij/?dir=assets";
        let mut params = FlakeRefParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_uri_params_nom_alt() {
        let uri = "github:zellij-org/zellij/?dir=assets&nar_hash=fakeHash256";
        let mut params = FlakeRefParameters::default();
        params.dir(Some("assets".into()));
        params.nar_hash(Some("fakeHash256".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
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
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_simple_path_params_nom() {
        let uri = "path:/home/kenji/.config/dotfiles/?dir=assets";
        let mut params = FlakeRefParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/.config/dotfiles/".into(),
            })
            .params(params)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_gitlab_simple() {
        let uri = "gitlab:veloren/veloren";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "veloren".into(),
                repo: "veloren".into(),
                ref_or_rev: None,
            })
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_gitlab_simple_ref_or_rev() {
        let uri = "gitlab:veloren/veloren/master";
        let parsed = parse_nix_uri(uri).unwrap();
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "veloren".into(),
                repo: "veloren".into(),
                ref_or_rev: Some("master".into()),
            })
            .clone();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_gitlab_simple_ref_or_rev_alt() {
        let uri = "gitlab:veloren/veloren/19742bb9300fb0be9fdc92f30766c95230a8a371";
        let parsed = crate::parser::parse_nix_uri(uri).unwrap();
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "veloren".into(),
                repo: "veloren".into(),
                ref_or_rev: Some("19742bb9300fb0be9fdc92f30766c95230a8a371".into()),
            })
            .clone();
        assert_eq!(flake_ref, parsed);
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
    #[test]
    fn parse_gitlab_simple_host_param() {
        let uri = "gitlab:openldap/openldap?host=git.openldap.org";
        let parsed = crate::parser::parse_nix_uri(uri).unwrap();
        let mut params = FlakeRefParameters::default();
        params.host(Some("git.openldap.org".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "openldap".into(),
                repo: "openldap".into(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        assert_eq!(flake_ref, parsed);
    }
    #[test]
    fn parse_git_and_https_simple() {
        let uri = "git+https://git.somehost.tld/user/path";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "git.somehost.tld/user/path".into(),
                r#type: UrlType::Https,
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_https_params() {
        let uri = "git+https://git.somehost.tld/user/path?ref=branch&rev=fdc8ef970de2b4634e1b3dca296e1ed918459a9e";
        let mut params = FlakeRefParameters::default();
        params.r#ref(Some("branch".into()));
        params.rev(Some("fdc8ef970de2b4634e1b3dca296e1ed918459a9e".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "git.somehost.tld/user/path".into(),
                r#type: UrlType::Https,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_file_params() {
        let uri = "git+file:///nix/nixpkgs?ref=upstream/nixpkgs-unstable";
        let mut params = FlakeRefParameters::default();
        params.r#ref(Some("upstream/nixpkgs-unstable".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "/nix/nixpkgs".into(),
                r#type: UrlType::File,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_file_simple() {
        let uri = "git+file:///nix/nixpkgs";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "/nix/nixpkgs".into(),
                r#type: UrlType::File,
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    // TODO: is this correct?
    // git+file:/home/user/forked-flake?branch=feat/myNewFeature
    fn parse_git_and_file_params_alt() {
        let uri = "git+file:///home/user/forked-flake?branch=feat/myNewFeature";
        let mut params = FlakeRefParameters::default();
        params.set_branch(Some("feat/myNewFeature".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "/home/user/forked-flake".into(),
                r#type: UrlType::File,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_github_simple_tag_non_alphabetic_params() {
        let uri = "github:smunix/MyST-Parser?ref=fix.hls-docutils";
        let mut params = FlakeRefParameters::default();
        params.set_ref(Some("fix.hls-docutils".to_owned()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "smunix".into(),
                repo: "MyST-Parser".into(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_github_simple_tag() {
        let uri = "github:cachix/devenv/v0.5";
        let mut params = FlakeRefParameters::default();
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "cachix".into(),
                repo: "devenv".into(),
                ref_or_rev: Some("v0.5".into()),
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_file_params_alt_branch() {
        let uri = "git+file:///home/user/forked-flake?branch=feat/myNewFeature";
        let mut params = FlakeRefParameters::default();
        params.set_branch(Some("feat/myNewFeature".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "/home/user/forked-flake".into(),
                r#type: UrlType::File,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_gitlab_with_host_params_alt() {
        let uri = "gitlab:fpottier/menhir/20201216?host=gitlab.inria.fr";
        let mut params = FlakeRefParameters::default();
        params.set_host(Some("gitlab.inria.fr".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitLab,
                owner: "fpottier".to_owned(),
                repo: "menhir".to_owned(),
                ref_or_rev: Some("20201216".to_owned()),
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_https_params_submodules() {
        let uri = "git+https://www.github.com/ocaml/ocaml-lsp?submodules=1";
        let mut params = FlakeRefParameters::default();
        params.set_submodules(Some("1".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "www.github.com/ocaml/ocaml-lsp".to_owned(),
                r#type: UrlType::Https,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_marcurial_and_https_simpe_uri() {
        let uri = "hg+https://www.github.com/ocaml/ocaml-lsp";
        let mut params = FlakeRefParameters::default();
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Mercurial {
                url: "www.github.com/ocaml/ocaml-lsp".to_owned(),
                r#type: UrlType::Https,
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    #[should_panic]
    fn parse_git_and_https_params_submodules_wrong_type() {
        let uri = "gt+https://www.github.com/ocaml/ocaml-lsp?submodules=1";
        let mut params = FlakeRefParameters::default();
        params.set_submodules(Some("1".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "www.github.com/ocaml/ocaml-lsp".to_owned(),
                r#type: UrlType::Https,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_git_and_file_shallow() {
        let uri = "git+file:/path/to/repo?shallow=1";
        let mut params = FlakeRefParameters::default();
        params.set_shallow(Some("1".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Git {
                url: "/path/to/repo".to_owned(),
                r#type: UrlType::File,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
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
        let uri = "sourcehut:~misterio/nix-colors";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: None,
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_simple_uri_sourcehut_rev() {
        let uri = "sourcehut:~misterio/nix-colors/main";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("main".to_owned()),
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_simple_uri_sourcehut_host_param() {
        let uri = "sourcehut:~misterio/nix-colors?host=git.example.org";
        let mut params = FlakeRefParameters::default();
        params.set_host(Some("git.example.org".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_simple_uri_sourcehut_ref() {
        let uri = "sourcehut:~misterio/nix-colors/182b4b8709b8ffe4e9774a4c5d6877bf6bb9a21c";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("182b4b8709b8ffe4e9774a4c5d6877bf6bb9a21c".to_owned()),
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_simple_uri_sourcehut_ref_params() {
        let uri =
            "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de?host=hg.sr.ht";
        let mut params = FlakeRefParameters::default();
        params.set_host(Some("hg.sr.ht".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn display_simple_sourcehut_uri_ref_or_rev() {
        let expected = "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de";
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
            })
            .to_string();
        assert_eq!(expected, flake_ref);
    }
    #[test]
    fn display_simple_sourcehut_uri_ref_or_rev_host_param() {
        let expected =
            "sourcehut:~misterio/nix-colors/21c1a380a6915d890d408e9f22203436a35bb2de?host=hg.sr.ht";
        let mut params = FlakeRefParameters::default();
        params.set_host(Some("hg.sr.ht".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::SourceHut,
                owner: "~misterio".to_owned(),
                repo: "nix-colors".to_owned(),
                ref_or_rev: Some("21c1a380a6915d890d408e9f22203436a35bb2de".to_owned()),
            })
            .params(params)
            .to_string();
        assert_eq!(expected, flake_ref);
    }
    #[test]
    fn display_simple_github_uri_ref() {
        let expected = "github:zellij-org/zellij?ref=main";
        let mut flake_attrs = FlakeRefParameters::default();
        flake_attrs.r#ref(Some("main".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .params(flake_attrs)
            .to_string();
        assert_eq!(flake_ref, expected);
    }
    #[test]
    fn display_simple_github_uri_rev() {
        let expected = "github:zellij-org/zellij?rev=b2df4e4e80e04cbb33a350f87717f4bd6140d298";
        let mut flake_attrs = FlakeRefParameters::default();
        flake_attrs.rev(Some("b2df4e4e80e04cbb33a350f87717f4bd6140d298".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitForge {
                platform: GitForgePlatform::GitHub,
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .params(flake_attrs)
            .to_string();
        assert_eq!(flake_ref, expected);
    }
    #[test]
    fn parse_simple_path_uri_indirect_absolute_without_prefix() {
        let uri = "/home/kenji/git";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/git".to_owned(),
            })
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
    }
    #[test]
    fn parse_simple_path_uri_indirect_absolute_without_prefix_with_params() {
        let uri = "/home/kenji/git?dir=dev";
        let mut params = FlakeRefParameters::default();
        params.set_dir(Some("dev".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::Path {
                path: "/home/kenji/git".to_owned(),
            })
            .params(params)
            .clone();
        let parsed: FlakeRef = uri.try_into().unwrap();
        assert_eq!(expected, parsed);
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
        let uri = "git+(:z";
        let expected = NixUriError::UnknownUrlType("(".into());
        let parsed: NixUriResult<FlakeRef> = uri.try_into();
        assert_eq!(expected, parsed.unwrap_err());
    }
    #[test]
    fn parse_github_missing_parameter() {
        let uri = "github:";
        let expected = NixUriError::MissingTypeParameter("github".into(), ("owner".into()));
        let parsed: NixUriResult<FlakeRef> = uri.try_into();
        assert_eq!(expected, parsed.unwrap_err());
    }
    #[test]
    fn parse_github_missing_parameter_repo() {
        let uri = "github:nixos/";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::MissingTypeParameter(
                "github".into(),
                ("repo".into())
            ))
        );
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
    }
    #[test]
    fn parse_empty_invalid_url() {
        let uri = "";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
    }
    #[test]
    fn parse_empty_trim_invalid_url() {
        let uri = "  ";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
    }
    #[test]
    fn parse_slash_trim_invalid_url() {
        let uri = "   /   ";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
    }
    #[test]
    fn parse_double_trim_invalid_url() {
        let uri = "   :   ";
        assert_eq!(
            uri.parse::<FlakeRef>(),
            Err(NixUriError::InvalidUrl(uri.into()))
        );
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
    //     let mut params = FlakeRefParameters::default();
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
