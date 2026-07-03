//! CLI 发布版本号解析与比较（`major.minor.patch` 与可选 `-beta.N`）

use anyhow::Result;
use std::cmp::Ordering;
use std::fmt::{self, Display};
use std::str::FromStr;
use winnow::Parser;
use winnow::ascii::digit1;
use winnow::combinator::{alt, opt, preceded, seq};
use winnow::error::{ContextError, ErrMode};
use winnow::prelude::*;

/// CLI / npm 发布版本号
///
/// - `1.0.125` — 正式版
/// - `1.0.125-beta.7` — beta 预发布版
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    /// `None` 为正式版；`Some(n)` 表示 `-beta.n`
    pub beta: Option<u32>,
}

impl ReleaseVersion {
    pub fn new(major: u32, minor: u32, patch: u32, beta: Option<u32>) -> Self {
        Self {
            major,
            minor,
            patch,
            beta,
        }
    }

    /// 解析版本字符串（可选 `v`/`V` 前缀，不含 `dev` git tag 前缀）
    pub fn parse(input: &str) -> Result<Self> {
        if input.is_empty() {
            return Err(anyhow::anyhow!("Version string cannot be empty"));
        }
        Self::parse_version(input)
            .map_err(|_| anyhow::anyhow!("Failed to parse release version: {input}"))
    }

    fn parse_version(input: &str) -> ModalResult<Self> {
        let mut input_slice = input;

        let (_, major, minor, patch, beta) = seq!(
            opt(alt(("v", "V"))),
            digit1.parse_to::<u32>(),
            preceded('.', digit1.parse_to::<u32>()),
            preceded('.', digit1.parse_to::<u32>()),
            opt(preceded("-beta.", digit1.parse_to::<u32>())),
        )
        .parse_next(&mut input_slice)?;

        if !input_slice.is_empty() {
            return Err(ErrMode::Cut(ContextError::default()));
        }

        Ok(Self::new(major, minor, patch, beta))
    }
}

impl FromStr for ReleaseVersion {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.minor.cmp(&other.minor) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.patch.cmp(&other.patch) {
            Ordering::Equal => {}
            ord => return ord,
        }

        match (&self.beta, &other.beta) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => a.cmp(b),
        }
    }
}

impl Display for ReleaseVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(beta) = self.beta {
            write!(f, "-beta.{beta}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    fn parse(s: &str) -> ReleaseVersion {
        ReleaseVersion::parse(s).unwrap_or_else(|e| panic!("parse {s}: {e}"))
    }

    #[test]
    fn test_parse_release_versions() {
        let v = parse("1.0.125");
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 0);
        assert_eq!(v.patch, 125);
        assert_eq!(v.beta, None);

        let v = parse("v1.0.125-beta.7");
        assert_eq!(v.patch, 125);
        assert_eq!(v.beta, Some(7));

        let v = parse("1.0.125-beta.7");
        assert_eq!(v.beta, Some(7));

        assert!(ReleaseVersion::parse("dev1.0.125-beta.7").is_err());
        assert!(ReleaseVersion::parse("1.0.125-rc.1").is_err());
        assert!(ReleaseVersion::parse("").is_err());
    }

    #[test]
    fn test_compare_release_versions() {
        assert_eq!(
            parse("v1.0.125-beta.7").cmp(&parse("v1.0.123")),
            Ordering::Greater
        );
        assert_eq!(
            parse("v1.0.125-beta.7").cmp(&parse("v1.0.125-beta.8")),
            Ordering::Less
        );
        assert_eq!(
            parse("v1.0.125-beta.7").cmp(&parse("v1.0.125")),
            Ordering::Less
        );
        assert_eq!(
            parse("v1.0.125").cmp(&parse("v1.0.125-beta.7")),
            Ordering::Greater
        );
        assert_eq!(
            parse("v1.0.125-beta.7").cmp(&parse("v1.0.125-beta.7")),
            Ordering::Equal
        );
    }

    #[test]
    fn test_display_release_version() {
        assert_eq!(parse("1.0.125").to_string(), "1.0.125");
        assert_eq!(parse("1.0.125-beta.7").to_string(), "1.0.125-beta.7");
    }
}
