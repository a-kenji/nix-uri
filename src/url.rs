use url::Url;

use crate::{FlakeRef, FlakeRefType, NixUriResult};

impl FlakeRef {
    /// Parse the [`FlakeRef`], by an arbitrary url scheme,
    /// for certain services its type can be inferred e.g. github,
    /// for others it will need an explicit type set.
    pub fn from_url(url: &Url) -> NixUriResult<FlakeRef> {
        let maybe_type = infer_from_url(url);

        if let Some(r#type) = maybe_type {
        }
        todo!();
    }
    /// Infer the type of the [`FlakeRef`] by its uri scheme
    pub(crate) fn infer_from_url(url: &Url) -> Option<FlakeRefType> {
        match url.host() {
            Some(host) => {
                match host {
                    ""

                }
            }
            None => None
        }
        None
    }
}

mod test {
    use super::*;

    #[test]
    fn parse_simple_gh_url() {
        let uri = "github:nixos/";
        let url = "https://github.com/nixpkgs/nixos";
        let expected = NixUriError::MissingTypeParameter("github".into(), ("repo".into()));
        let parsed: NixUriResult<FlakeRef> = uri.try_into();
        assert_eq!(expected, parsed.unwrap_err());
    }

    #[test]
    fn parse_simple_uri_attr_nom() {
        let uri = "github:zellij-org/zellij?dir=assets";
        let mut params = FlakeRefParameters::default();
        params.dir(Some("assets".into()));
        let flake_ref = FlakeRef::default()
            .r#type(FlakeRefType::GitHub {
                owner: "zellij-org".into(),
                repo: "zellij".into(),
                ref_or_rev: None,
            })
            .params(params)
            .clone();
        let parsed = parse_nix_uri(uri).unwrap();
        assert_eq!(flake_ref, parsed);
    }
}
