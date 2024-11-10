use winnow::{PResult, Parser};

use crate::{error::NixUriError, flakeref::FlakeRef};

// pub(crate) fn parse_params(input: &mut &str) -> PResult<Option<LocationParameters>> {
//
//     let maybe_flake_type = opt(take_until(0.., "?")).parse_next(input)?;
//
//     if let Some(_flake_type) = maybe_flake_type {
//         // discard leading "?"
//         let _ = any(input)?;
//         // TODO: is this input really not needed?
//         let param_values: BTreeMap<&str, &str> = repeat(
//             0..11,
//             separated_pair(take_until(0.., "="), "=", alt((take_until(0.., "&"), rest))),
//         )
//         .parse_next(input)?;
//
//         let mut params = LocationParameters::default();
//         for (param, value) in param_values {
//             // param can start with "&"
//             // TODO: actual error handling instead of unwrapping
//             // TODO: allow check of the parameters
//             if let Ok(param) = param.parse() {
//                 match param {
//                     LocationParamKeys::Dir => params.set_dir(Some(value.into())),
//                     LocationParamKeys::NarHash => params.set_nar_hash(Some(value.into())),
//                     LocationParamKeys::Host => params.set_host(Some(value.into())),
//                     LocationParamKeys::Ref => params.set_ref(Some(value.into())),
//                     LocationParamKeys::Rev => params.set_rev(Some(value.into())),
//                     LocationParamKeys::Branch => params.set_branch(Some(value.into())),
//                     LocationParamKeys::Submodules => params.set_submodules(Some(value.into())),
//                     LocationParamKeys::Shallow => params.set_shallow(Some(value.into())),
//                     LocationParamKeys::Arbitrary(param) => {
//                         params.add_arbitrary((param, value.into()));
//                     }
//                 }
//             }
//         }
//         Ok(Some(params))
//     } else {
//         Ok(None)
//     }
// }

pub(crate) fn parse_nix_uri(input: &mut &str) -> Result<FlakeRef, NixUriError> {
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
        return Err(NixUriError::InvalidUrl(input.to_string()));
    }
    FlakeRef::parse(input).map_err(NixUriError::CtxError)
}

// /// Parses the raw-string describing the transport type out of: `+type`
// pub(crate) fn parse_from_transport_type<'i>(input: &mut &'i str) -> PResult<&'i str> {
//     let _rest = take_until(0.., "+").parse_next(input)?;
//     let _ = any(input)?;
//     Ok(input)
// }

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

// // Parse the transport type itself
// pub(crate) fn parse_transport_type(input: &mut &str) -> Result<TransportLayer, NixUriError> {
//     let input = parse_from_transport_type(input)?;
//     TryInto::<TransportLayer>::try_into(input)
// }

pub(crate) fn parse_sep<'i>(input: &mut &'i str) -> PResult<&'i str> {
    "://".parse_next(input)
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
