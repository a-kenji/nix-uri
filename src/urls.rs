use nom::Finish;
use url::Url;

use crate::{
    FlakeRef, FlakeRefType, GitForge, LocationParameters, NixUriError, NixUriResult,
    parser::is_tarball,
};

pub struct UrlWrapper {
    url: Url,
    infer_type: bool,
    explicit_type: FlakeRefType,
}
impl TryFrom<&str> for UrlWrapper {
    type Error = NixUriError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let url = Url::parse(input)?;
        Ok(Self::new(url))
    }
}

impl UrlWrapper {
    pub(crate) const fn new(url: Url) -> Self {
        Self {
            url,
            infer_type: true,
            explicit_type: FlakeRefType::None,
        }
    }
    pub fn infer_type(&mut self, infer_type: bool) -> &mut Self {
        self.infer_type = infer_type;
        self
    }
    pub fn explicit_type(&mut self, explicit_type: FlakeRefType) -> &mut Self {
        self.explicit_type = explicit_type;
        self
    }

    /// ```rust
    /// use nix_uri::urls::UrlWrapper;
    /// let http = "https://github.com/nixos/nixpkgs";
    /// let forge = "github:nixos/nixpkgs";
    /// let http_parsed = UrlWrapper::convert_or_parse(http);
    /// let forge_parsed = UrlWrapper::convert_or_parse(forge);
    /// // assert_eq!(http_parsed, forge_parsed);
    /// ```
    pub fn convert_or_parse(input: &str) -> NixUriResult<FlakeRef> {
        // If default parsing fails, it might still be a `nix-uri`.
        let url = Self::try_from(input).ok();

        if is_tarball(input) {
            return input.parse();
        }

        if let Some(url) = url {
            match url.url.host() {
                Some(host) => {
                    let flake_ref_type = url.type_from_host(&host.to_string())?;
                    let mut flake_ref = FlakeRef::default();
                    flake_ref.r#type(flake_ref_type);
                    if let Some(query) = url.url.query() {
                        let (_, params) = LocationParameters::parse(query)
                            .finish()
                            .map_err(NixUriError::from)?;
                        if params != LocationParameters::default() {
                            flake_ref.params(params);
                        }
                    }
                    Ok(flake_ref.clone())
                }
                None => input.parse(),
            }
        } else {
            input.parse()
        }
    }
    fn type_from_host(&self, input: &str) -> NixUriResult<FlakeRefType> {
        match input {
            "github.com" => {
                let segments = self
                    .url
                    .path_segments()
                    .map(std::iter::Iterator::collect::<Vec<_>>)
                    .ok_or_else(|| {
                        NixUriError::Error(format!("Error parsing from host: {}", input))
                    })?;
                let ref_or_rev = if segments.len() > 2 {
                    Some(segments[2..].join("/"))
                } else {
                    None
                };

                if segments.len() < 2 {
                    return Err(NixUriError::Error(format!(
                        "Error parsing from host: {}",
                        input
                    )));
                }

                Ok(FlakeRefType::GitForge(GitForge {
                    platform: crate::flakeref::GitForgePlatform::GitHub,
                    owner: segments[0].to_string(),
                    repo: segments[1].to_string(),
                    ref_or_rev,
                }))
            }
            _ => Ok(FlakeRefType::None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn simple_url_conversion() {
        let url = "https://github.com/nixos/nixpkgs";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: crate::flakeref::GitForgePlatform::GitHub,
                owner: "nixos".into(),
                repo: "nixpkgs".into(),
                ref_or_rev: None,
            }))
            .clone();
        assert_eq!(UrlWrapper::convert_or_parse(url).unwrap(), expected);
    }
    #[test]
    fn simple_url_conversion_with_param() {
        let url = "https://github.com/nixos/nixpkgs?dir=foo";
        let mut params = LocationParameters::default();
        params.dir(Some("foo".into()));
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: crate::flakeref::GitForgePlatform::GitHub,
                owner: "nixos".into(),
                repo: "nixpkgs".into(),
                ref_or_rev: None,
            }))
            .params(params)
            .clone();
        assert_eq!(UrlWrapper::convert_or_parse(url).unwrap(), expected);
    }
    #[test]
    fn simple_url_conversion_with_flake() {
        let url = "https://github.com/nixos/nixpkgs#fizz";
        let expected = FlakeRef::default()
            .r#type(FlakeRefType::GitForge(GitForge {
                platform: crate::flakeref::GitForgePlatform::GitHub,
                owner: "nixos".into(),
                repo: "nixpkgs".into(),
                ref_or_rev: None,
            }))
            .clone();
        assert_eq!(UrlWrapper::convert_or_parse(url).unwrap(), expected);
    }
    #[test]
    fn check_tarball_uri_conversion() {
        let filename = "https://github.com/NixOS/patchelf/archive/master.tar.gz";
        assert!(is_tarball(filename));
    }
}
