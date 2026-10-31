use std::borrow::Cow;

use crate::{
    error::{NixUriError, NixUriResult},
    flakeref::{
        RenderUrl,
        attrs::{AttrReader, Attrs},
        forge::render_rev,
        validators::{is_sha1_hex, validated_ref_name},
    },
};

/// A flake in the registry (`flake:nixpkgs`, `nixpkgs/nixos-25.05`).
///
/// A ref and a revision are only read from the path, `id[/ref][/rev]`. The query of a
/// `flake:` URL is ignored except for `dir`. `nar_hash` is only set from attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Indirect {
    pub id: String,
    pub ref_: Option<String>,
    pub rev: Option<String>,
    pub nar_hash: Option<String>,
}

const MAX_SEGMENTS: usize = 3;

/// Returns `true` if `s` is a flake id: an ASCII letter followed by ASCII alphanumerics,
/// `-` and `_`.
pub(crate) fn is_flake_id(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Indirect {
    pub fn new(id: String) -> Self {
        Self {
            id,
            ref_: None,
            rev: None,
            nar_hash: None,
        }
    }

    /// Build an indirect reference from its non-empty path segments (`id[/ref[/rev]]`).
    ///
    /// With two segments, the second is a revision when it is 40 hex digits and a ref
    /// otherwise. With three, the second must be a ref and the third a 40-hex SHA-1
    /// commit. A revision is lower-cased.
    pub(crate) fn from_segments(segments: &[&str], raw_input: &str) -> NixUriResult<Self> {
        if segments.len() > MAX_SEGMENTS {
            return Err(NixUriError::TooManyIndirectSegments {
                count: segments.len(),
            });
        }
        let mut indirect = match segments.first() {
            Some(id) if is_flake_id(id) => Self::new((*id).to_string()),
            _ => return Err(NixUriError::InvalidUrl(raw_input.into())),
        };
        match segments {
            [_, value] if is_sha1_hex(value) => indirect.rev = Some(value.to_ascii_lowercase()),
            [_, value] => indirect.ref_ = Some(validated_ref_name(value)?),
            [_, ref_, rev] => {
                if !is_sha1_hex(rev) {
                    return Err(NixUriError::InvalidValue {
                        field: "rev",
                        reason: "expected a 40-hex commit hash in the third indirect segment"
                            .to_string(),
                    });
                }
                indirect.ref_ = Some(validated_ref_name(ref_)?);
                indirect.rev = Some(rev.to_ascii_lowercase());
            }
            _ => {}
        }
        Ok(indirect)
    }

    pub(crate) fn from_attrs(mut attrs: AttrReader<'_>) -> NixUriResult<Self> {
        let id = attrs.required_string("id")?;
        if !is_flake_id(&id) {
            return Err(NixUriError::InvalidValue {
                field: "id",
                reason: "expected a flake id".to_string(),
            });
        }
        let indirect = Self {
            id,
            ref_: attrs
                .string("ref")?
                .map(|r| validated_ref_name(&r))
                .transpose()?,
            rev: attrs.string("rev")?,
            nar_hash: attrs.string("narHash")?,
        };
        attrs.finish()?;
        Ok(indirect)
    }

    pub(crate) fn write_attrs(&self, attrs: &mut Attrs) {
        attrs.insert("type", "indirect");
        attrs.insert("id", &self.id);
        attrs.set_opt("ref", self.ref_.as_ref());
        attrs.set_opt("rev", self.rev.as_ref());
        attrs.set_opt("narHash", self.nar_hash.as_ref());
    }

    /// `flake:<id>[/<ref>][/<rev>]`. `narHash` is not written.
    pub(crate) fn render(&self) -> RenderUrl<'_> {
        let mut segments = vec![Cow::from(&self.id)];
        segments.extend(self.ref_.as_ref().map(Cow::from));
        segments.extend(self.rev.as_deref().map(|rev| Cow::from(render_rev(rev))));
        RenderUrl::from_segments("flake", segments)
    }
}
