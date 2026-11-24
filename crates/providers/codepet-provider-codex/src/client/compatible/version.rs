use super::super::{CodexAppServerError, CodexAppServerSession};
use semver::Version;

impl CodexAppServerSession {
    pub(in crate::client) fn version_at_most(
        &self,
        upper: &str,
    ) -> Result<bool, CodexAppServerError> {
        let actual = self.harness_version().ok_or_else(|| {
            CodexAppServerError::Protocol(
                "Codex initialize did not identify the running version".into(),
            )
        })?;
        compare(&actual, upper)
    }
}

fn compare(actual: &str, upper: &str) -> Result<bool, CodexAppServerError> {
    let parse = |value: &str| {
        Version::parse(value).map_err(|_| {
            CodexAppServerError::Protocol(format!("invalid Codex compatibility version: {value}"))
        })
    };
    // Build metadata does not affect SemVer precedence.
    Ok(parse(actual)?.cmp_precedence(&parse(upper)?).is_le())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_complete_prereleases_and_leaves_latest_branch_open() {
        let upper = "0.166.0-alpha.5.1";
        for version in [
            "0.152.0",
            "0.166.0-alpha.5",
            upper,
            "0.166.0-alpha.5.1+local",
        ] {
            assert!(compare(version, upper).unwrap(), "{version}");
        }
        for version in [
            "0.166.0-alpha.5.2",
            "0.166.0-alpha.13",
            "0.166.0",
            "0.171.0-alpha.13",
            "99.0.0",
        ] {
            assert!(!compare(version, upper).unwrap(), "{version}");
        }
        for version in ["", "1", "unknown"] {
            assert!(compare(version, upper).is_err());
        }
    }
}
