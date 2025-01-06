use nom::{
    branch::alt,
    bytes::complete::is_not,
    character::complete::{alphanumeric1, char},
    combinator::{cut, opt, recognize},
    multi::separated_list1,
    sequence::{delimited, preceded},
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::IErr;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttributePath {
    pub split: Vec<String>,
}

impl AttributePath {
    pub(crate) fn parse(input: &str) -> IResult<&str, Self, IErr<&str>> {
        let (rest, split): (&str, Vec<&str>) = separated_list1(
            char('.'),
            alt((
                alphanumeric1,
                recognize(delimited(char('"'), is_not("\""), char('"'))),
            )),
        )(input)?;
        let split = split.iter().map(ToString::to_string).collect();
        let me = Self { split };
        Ok((rest, me))
    }
    pub(crate) fn try_parse_preceded(input: &str) -> IResult<&str, Option<Self>, IErr<&str>> {
        opt(preceded(char('#'), cut(AttributePath::parse)))(input)
    }
}
#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn empty() {
        let _ = AttributePath::parse("").unwrap_err();
        let non = AttributePath::try_parse_preceded("").unwrap().1;
        assert_eq!(None, non);
        let _ = AttributePath::try_parse_preceded("#").unwrap_err();
    }

    #[test]
    fn singl() {
        let foo_res = AttributePath::try_parse_preceded("#foo")
            .unwrap()
            .1
            .unwrap();

        assert_eq!(
            AttributePath {
                split: vec!["foo".to_string()]
            },
            foo_res
        );
        let foo_res = AttributePath::try_parse_preceded("#foo.bar")
            .unwrap()
            .1
            .unwrap();
        assert_eq!(
            AttributePath {
                split: vec!["foo".to_string(), "bar".to_string()]
            },
            foo_res
        );
    }
    #[test]
    fn quoted() {
        let foo_res = AttributePath::try_parse_preceded("#\"foo\"")
            .unwrap()
            .1
            .unwrap();

        assert_eq!(
            AttributePath {
                split: vec!["\"foo\"".to_string()]
            },
            foo_res
        );
        let foo_res = AttributePath::try_parse_preceded(r##"#foo."bar""##)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(
            AttributePath {
                split: vec!["foo".to_string(), "\"bar\"".to_string()]
            },
            foo_res
        );
    }
}
