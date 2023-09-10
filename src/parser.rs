use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    character::complete::anychar,
    combinator::{opt, rest},
    multi::many_m_n,
    IResult,
};

use crate::{
    error::NixUriError,
    flakeref::{FlakeRef, FlakeRefParam, FlakeRefParameters, FlakeRefType, UrlType},
};

/// Parses content of the form `/owner/repo/ref_or_rev`
/// into a `vec![owner, repo, ref_or_rev]`.
pub(crate) fn parse_owner_repo_ref(input: &str) -> IResult<&str, Vec<&str>> {
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
pub(crate) fn parse_params(input: &str) -> IResult<&str, Option<FlakeRefParameters>> {
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

        let mut params = FlakeRefParameters::default();
        for (param, value) in param_values {
            // param can start with "&"
            // TODO: actual error handling instead of unwrapping
            match param.parse().unwrap() {
                FlakeRefParam::Dir => params.set_dir(Some(value.into())),
                FlakeRefParam::NarHash => params.set_nar_hash(Some(value.into())),
                FlakeRefParam::Host => params.set_host(Some(value.into())),
                FlakeRefParam::Ref => params.set_ref(Some(value.into())),
                FlakeRefParam::Rev => params.set_rev(Some(value.into())),
                FlakeRefParam::Branch => params.set_branch(Some(value.into())),
                FlakeRefParam::Submodules => params.set_submodules(Some(value.into())),
                FlakeRefParam::Shallow => params.set_shallow(Some(value.into())),
            }
        }
        Ok((flake_type, Some(params)))
    } else {
        Ok((input, None))
    }
}

pub(crate) fn parse_nix_uri(input: &str) -> IResult<&str, FlakeRef> {
    let (input, params) = parse_params(input)?;
    let mut flake_ref = FlakeRef::default();
    let (input, flake_ref_type) = FlakeRefType::parse_type(input)?;
    flake_ref.r#type(flake_ref_type);
    if let Some(params) = params {
        flake_ref.params(params);
    }

    Ok((input, flake_ref))
}

/// Parses the url raw url type out of: `+type`
pub(crate) fn parse_from_url_type(input: &str) -> IResult<&str, &str> {
    let (input, rest) = take_until("+")(input)?;
    let (input, _) = anychar(input)?;
    Ok((rest, input))
}

// Parse the url type itself
pub(crate) fn parse_url_type(input: &str) -> Result<UrlType, NixUriError> {
    let (_, input) = parse_from_url_type(input)?;
    TryInto::<UrlType>::try_into(input)
}