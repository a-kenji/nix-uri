use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    character::complete::anychar,
    combinator::{opt, rest},
    error::{context, VerboseError},
    multi::many_m_n,
    IResult,
};

use crate::{
    error::{NixUriError, NixUriResult},
    flakeref::{FlakeRef, LocationParamKeys, LocationParameters, TransportLayer},
};

// TODO: use a param-specific parser, handle the inversion specificially
/// Take all that is behind the "?" tag
/// Return everything prior as not parsed
pub(crate) fn parse_params(input: &str) -> IResult<&str, Option<LocationParameters>> {
    use nom::sequence::separated_pair;

    // This is the inverse of the general control flow
    let (input, maybe_flake_type) = opt(take_until("?"))(input)?;

    if let Some(flake_type) = maybe_flake_type {
        // discard leading "?"
        let (input, _) = anychar(input)?;
        // TODO: is this input really not needed?
        let (_input, param_values) = many_m_n(
            0,
            11,
            separated_pair(take_until("="), tag("="), alt((take_until("&"), rest))),
        )(input)?;

        let mut params = LocationParameters::default();
        for (param, value) in param_values {
            // param can start with "&"
            // TODO: actual error handling instead of unwrapping
            // TODO: allow check of the parameters
            if let Ok(param) = param.parse() {
                match param {
                    LocationParamKeys::Dir => params.set_dir(Some(value.into())),
                    LocationParamKeys::NarHash => params.set_nar_hash(Some(value.into())),
                    LocationParamKeys::Host => params.set_host(Some(value.into())),
                    LocationParamKeys::Ref => params.set_ref(Some(value.into())),
                    LocationParamKeys::Rev => params.set_rev(Some(value.into())),
                    LocationParamKeys::Branch => params.set_branch(Some(value.into())),
                    LocationParamKeys::Submodules => params.set_submodules(Some(value.into())),
                    LocationParamKeys::Shallow => params.set_shallow(Some(value.into())),
                    LocationParamKeys::Arbitrary(param) => {
                        params.add_arbitrary((param, value.into()));
                    }
                }
            }
        }
        Ok((flake_type, Some(params)))
    } else {
        Ok((input, None))
    }
}

pub(crate) fn parse_nix_uri(input: &str) -> NixUriResult<FlakeRef> {
    // fluent_uri::Uri::parse(input)?;
    // Basic sanity checks
    if input.trim().is_empty()
        || (input.trim() == "/")
        || (input.trim() == ":")
        || (input.trim() == "?")
        || (!input.is_ascii())
        || (!input.chars().all(|c| !c.is_control()))
        || (!input.chars().all(|c| !c.is_ascii_control()))
        || (input.ends_with(char::is_whitespace))
        || (input.starts_with(char::is_whitespace))
    {
        return Err(NixUriError::InvalidUrl(input.into()));
    }

    // let (input, params) = parse_params(input)?;
    match FlakeRef::parse(input) {
        Ok((_, res)) => Ok(res),
        Err(e) => Err(NixUriError::NomParseError(todo!("{:?}", e))),
    }
}

/// Parses the raw-string describing the transport type out of: `+type`
pub(crate) fn parse_from_transport_type(input: &str) -> IResult<&str, &str> {
    let (input, rest) = take_until("+")(input)?;
    let (input, _) = anychar(input)?;
    Ok((rest, input))
}

pub(crate) fn is_tarball(input: &str) -> bool {
    let valid_extensions = &[
        ".tar", ".gz", ".bz2", ".xz", ".zip", ".tar.bz2", ".tar.zst", ".tgz", ".tar.gz", ".tar.xz",
    ];
    valid_extensions.iter().any(|&ext| input.ends_with(ext))
}

#[allow(unused)]
pub(crate) fn is_file(input: &str) -> bool {
    !is_tarball(input)
}

// Parse the transport type itself
pub(crate) fn parse_transport_type(input: &str) -> Result<TransportLayer, NixUriError> {
    let (_, input) = parse_from_transport_type(input)?;
    TryInto::<TransportLayer>::try_into(input)
}

pub(crate) fn parse_sep(input: &str) -> IResult<&str, &str, VerboseError<&str>> {
    context("Expected `://`", tag("://"))(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn check_tarball() {
        let filename = "example.tar.gz";
        assert!(is_tarball(filename));
    }
    #[test]
    fn check_tarball_uri() {
        let filename = "https://github.com/NixOS/patchelf/archive/master.tar.gz";
        assert!(is_tarball(filename));
    }
    #[test]
    fn check_file_uri() {
        let filename = "https://github.com/NixOS/patchelf/";
        assert!(is_file(filename));
    }
    #[test]
    fn check_file() {
        let filename = "example";
        assert!(is_file(filename));
    }
}
