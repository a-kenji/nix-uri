use std::{fmt::Display, path::Path};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    combinator::{opt, rest},
    multi::many_m_n,
    sequence::separated_pair,
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{NixUriError, NixUriResult},
    parser::parse_url_type,
};

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, serde(deny_unknown_fields))]
pub struct FlakeRefParameters {
    /// The subdirectory of the flake in which flake.nix is located. This parameter
    /// enables having multiple flakes in a repository or tarball. The default is the
    /// root directory of the flake.
    dir: Option<String>,
    /// The hash of the NAR serialisation (in SRI format) of the contents of the flake.
    /// This is useful for flake types such as tarballs that lack a unique content
    /// identifier such as a Git commit hash.
    #[serde(rename = "narHash")]
    nar_hash: Option<String>,
    /// A Git or Mercurial commit hash.
    rev: Option<String>,
    ///  A Git or Mercurial branch or tag name.
    r#ref: Option<String>,
    branch: Option<String>,
    submodules: Option<String>,
    shallow: Option<String>,
    // Only available to certain types
    host: Option<String>,
    // Not available to user
    #[serde(rename = "revCount")]
    rev_count: Option<String>,
    // Not available to user
    #[serde(rename = "lastModified")]
    last_modified: Option<String>,
    /// Arbitrary uri parameters will be allowed during initial parsing
    /// in case they should be checked for known types run `self.check()`
    arbitrary: Vec<(String, String)>,
}

// TODO: convert into macro!
// or have params in a vec of tuples? with param and option<string>
impl Display for FlakeRefParameters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut res = String::new();
        if let Some(dir) = &self.dir {
            res.push_str("dir=");
            res.push_str(dir);
        }
        if let Some(branch) = &self.branch {
            if !res.is_empty() {
                res.push('?');
            }
            res.push_str("branch=");
            res.push_str(branch);
        }
        if let Some(host) = &self.host {
            if !res.is_empty() {
                res.push('?');
            }
            res.push_str("host=");
            res.push_str(host);
        }
        if let Some(r#ref) = &self.r#ref {
            if !res.is_empty() {
                res.push('?');
            }
            res.push_str("ref=");
            res.push_str(r#ref);
        }
        if let Some(rev) = &self.rev {
            if !res.is_empty() {
                res.push('?');
            }
            res.push_str("rev=");
            res.push_str(rev);
        }
        write!(f, "{res}")
    }
}

impl FlakeRefParameters {
    pub fn parse(input: &str) -> IResult<&str, Self> {
        let (input, _) = tag("?")(input)?;

        let (rest, param_values) = many_m_n(
            0,
            11,
            separated_pair(
                take_until("="),
                tag("="),
                alt((take_until("&"), take_until("#"), rest)),
            ),
        )(input)?;

        let mut params = FlakeRefParameters::default();
        for (param, value) in param_values {
            // param can start with "&"
            // TODO: actual error handling instead of unwrapping
            // TODO: allow check of the parameters
            if let Ok(param) = param.parse() {
                match param {
                    FlakeRefParamKeys::Dir => params.set_dir(Some(value.into())),
                    FlakeRefParamKeys::NarHash => params.set_nar_hash(Some(value.into())),
                    FlakeRefParamKeys::Host => params.set_host(Some(value.into())),
                    FlakeRefParamKeys::Ref => params.set_ref(Some(value.into())),
                    FlakeRefParamKeys::Rev => params.set_rev(Some(value.into())),
                    FlakeRefParamKeys::Branch => params.set_branch(Some(value.into())),
                    FlakeRefParamKeys::Submodules => params.set_submodules(Some(value.into())),
                    FlakeRefParamKeys::Shallow => params.set_shallow(Some(value.into())),
                    FlakeRefParamKeys::Arbitrary(param) => {
                        params.add_arbitrary((param, value.into()))
                    }
                }
            }
        }
        Ok((rest, params))
    }
    pub fn dir(&mut self, dir: Option<String>) -> &mut Self {
        self.dir = dir;
        self
    }

    pub fn nar_hash(&mut self, nar_hash: Option<String>) -> &mut Self {
        self.nar_hash = nar_hash;
        self
    }

    pub fn host(&mut self, host: Option<String>) -> &mut Self {
        self.host = host;
        self
    }
    pub fn rev(&mut self, rev: Option<String>) -> &mut Self {
        self.rev = rev;
        self
    }
    pub fn r#ref(&mut self, r#ref: Option<String>) -> &mut Self {
        self.r#ref = r#ref;
        self
    }

    pub fn set_dir(&mut self, dir: Option<String>) {
        self.dir = dir;
    }

    pub fn set_nar_hash(&mut self, nar_hash: Option<String>) {
        self.nar_hash = nar_hash;
    }

    pub fn set_rev(&mut self, rev: Option<String>) {
        self.rev = rev;
    }

    pub fn set_ref(&mut self, r#ref: Option<String>) {
        self.r#ref = r#ref;
    }

    pub fn set_host(&mut self, host: Option<String>) {
        self.host = host;
    }

    pub fn rev_count_mut(&mut self) -> &mut Option<String> {
        &mut self.rev_count
    }

    pub fn set_branch(&mut self, branch: Option<String>) {
        self.branch = branch;
    }

    pub fn set_submodules(&mut self, submodules: Option<String>) {
        self.submodules = submodules;
    }

    pub fn set_shallow(&mut self, shallow: Option<String>) {
        self.shallow = shallow;
    }
    pub fn add_arbitrary(&mut self, arbitrary: (String, String)) {
        self.arbitrary.push(arbitrary);
    }
    pub fn get_rev(&self) -> Option<&String> {
        self.rev.as_ref()
    }
    pub fn get_ref(&self) -> Option<&String> {
        self.r#ref.as_ref()
    }
}

pub enum FlakeRefParamKeys {
    Dir,
    NarHash,
    Host,
    Ref,
    Rev,
    Branch,
    Submodules,
    Shallow,
    Arbitrary(String),
}

impl std::str::FromStr for FlakeRefParamKeys {
    type Err = NixUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use FlakeRefParamKeys::*;
        match s {
            "dir" | "&dir" => Ok(Dir),
            "nar_hash" | "&nar_hash" => Ok(NarHash),
            "host" | "&host" => Ok(Host),
            "rev" | "&rev" => Ok(Rev),
            "ref" | "&ref" => Ok(Ref),
            "branch" | "&branch" => Ok(Branch),
            "submodules" | "&submodules" => Ok(Submodules),
            "shallow" | "&shallow" => Ok(Shallow),
            arbitrary => Ok(Arbitrary(arbitrary.into())),
            // unknown => Err(NixUriError::UnknownUriParameter(unknown.into())),
        }
    }
}

#[cfg(test)]
mod inc_parse {
    use super::*;
    #[test]
    fn empty() {
        let expected = FlakeRefParameters::default();
        let in_str = "?";
        let (rest, output) = FlakeRefParameters::parse(in_str).unwrap();
        assert_eq!("", rest);
        assert_eq!(output, expected);
    }
    #[test]
    fn empty_hash_terminated() {
        let expected = FlakeRefParameters::default();
        let in_str = "?#";
        let (rest, output) = FlakeRefParameters::parse(in_str).unwrap();
        assert_eq!("#", rest);
        assert_eq!(output, expected);
    }
    #[test]
    fn dir() {
        let mut expected = FlakeRefParameters::default();
        expected.dir(Some("foo".to_string()));

        let in_str = "?dir=foo";
        let (rest, output) = FlakeRefParameters::parse(in_str).unwrap();
        assert_eq!("", rest);
        assert_eq!(output, expected);

        let in_str = "?&dir=foo";
        let (rest, output) = FlakeRefParameters::parse(in_str).unwrap();
        assert_eq!("", rest);
        assert_eq!(output, expected);
        let in_str = "?dir=&dir=foo";
        let (rest, output) = FlakeRefParameters::parse(in_str).unwrap();
        assert_eq!("", rest);
        assert_eq!(output, expected);

        expected.dir(Some("".to_string()));
        let in_str = "?dir=";
        let (rest, output) = FlakeRefParameters::parse(in_str).unwrap();
        assert_eq!("", rest);
        assert_eq!(output, expected);
    }
    #[test]
    fn dir_hash_term() {
        let mut expected = FlakeRefParameters::default();
        expected.dir(Some("foo".to_string()));

        let in_str = "?dir=foo#fizz";
        let (rest, output) = FlakeRefParameters::parse(in_str).unwrap();
        assert_eq!("#fizz", rest);
        assert_eq!(output, expected);

        let in_str = "?&dir=foo#fizz";
        let (rest, output) = FlakeRefParameters::parse(in_str).unwrap();
        assert_eq!("#fizz", rest);
        assert_eq!(output, expected);
        let in_str = "?dir=&dir=foo#fizz";
        let (rest, output) = FlakeRefParameters::parse(in_str).unwrap();
        assert_eq!("#fizz", rest);
        assert_eq!(output, expected);

        expected.dir(Some("".to_string()));
        let in_str = "?dir=#fizz";
        let (rest, output) = FlakeRefParameters::parse(in_str).unwrap();
        assert_eq!("#fizz", rest);
        assert_eq!(output, expected);
    }
}
