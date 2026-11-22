//! `cargo xtask check-tag <tag>`: the release tag must name the version of the workspace (spec 021). The pipeline
//! runs it before building, so that a tag never publishes an exe whose version mark says something else.

/// Channel of a release, from the form of its version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// `X.Y.Z`.
    Stable,
    /// `X.Y.Z-rc.N`, offered only to clients that receive previews.
    Beta,
}

impl Channel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "canal stable",
            Self::Beta => "préversion, canal bêta",
        }
    }
}

/// A numeric identifier of semver: digits, without a leading zero.
fn is_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) && (text == "0" || !text.starts_with('0'))
}

/// The channel of a release version: only `X.Y.Z` and `X.Y.Z-rc.N` are published.
pub fn release_channel(version: &str) -> Option<Channel> {
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 || !parts.iter().all(|p| is_number(p)) {
        return None;
    }
    match pre {
        None => Some(Channel::Stable),
        Some(pre) => pre.strip_prefix("rc.").filter(|n| is_number(n)).map(|_| Channel::Beta),
    }
}

/// Checks that `tag` is `v` followed by `version`, the version of the workspace, and that this version is one the
/// pipeline publishes.
pub fn check_tag(tag: &str, version: &str) -> Result<Channel, String> {
    let channel = release_channel(version).ok_or_else(|| {
        format!("la version du workspace « {version} » n'est pas publiable : X.Y.Z ou X.Y.Z-rc.N attendu")
    })?;
    let Some(named) = tag.strip_prefix('v') else {
        return Err(format!(
            "le tag « {tag} » doit commencer par « v » (v{version} attendu)"
        ));
    };
    if named != version {
        return Err(format!(
            "le tag « {tag} » ne correspond pas à la version du workspace {version} (v{version} attendu) : changer \
             `version` dans le Cargo.toml racine, ou créer le bon tag"
        ));
    }
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exigence : EX-021-02
    #[test]
    fn equal_tag_and_version_are_accepted() {
        assert_eq!(check_tag("v0.1.0", "0.1.0"), Ok(Channel::Stable));
        assert_eq!(check_tag("v1.20.3", "1.20.3"), Ok(Channel::Stable));
    }

    // Exigence : EX-021-02
    #[test]
    fn a_different_tag_is_refused() {
        for (tag, version) in [
            ("v0.1.1", "0.1.0"),
            ("v0.1", "0.1.0"),
            ("v0.1.0.0", "0.1.0"),
            ("0.1.0", "0.1.0"),
            ("V0.1.0", "0.1.0"),
            ("vv0.1.0", "0.1.0"),
            ("v0.1.0 ", "0.1.0"),
            ("", "0.1.0"),
            ("v0.1.0-rc.1", "0.1.0"),
            ("v0.1.0", "0.1.0-rc.1"),
        ] {
            let refused = check_tag(tag, version);
            assert!(refused.is_err(), "{tag} / {version}");
        }
        let message = check_tag("v0.2.0", "0.1.0").unwrap_err();
        assert!(message.contains("v0.1.0 attendu"), "{message}");
    }

    // Exigence : EX-021-02
    #[test]
    fn a_preview_goes_to_the_beta_channel() {
        assert_eq!(check_tag("v0.2.0-rc.1", "0.2.0-rc.1"), Ok(Channel::Beta));
        assert_eq!(check_tag("v0.2.0-rc.12", "0.2.0-rc.12"), Ok(Channel::Beta));
        assert!(check_tag("v0.2.0-rc.2", "0.2.0-rc.1").is_err());
    }

    // Exigence : EX-021-02
    #[test]
    fn only_releases_and_release_candidates_are_published() {
        for version in [
            "0.2.0-beta.1",
            "0.2.0-rc",
            "0.2.0-rc.01",
            "0.2.0-rc.1.2",
            "0.2.0+build.5",
            "01.2.0",
            "1.2",
            "1.2.x",
            "",
        ] {
            assert_eq!(release_channel(version), None, "{version}");
            assert!(check_tag(&format!("v{version}"), version).is_err(), "{version}");
        }
    }

    #[test]
    fn the_workspace_version_is_publishable() {
        assert!(release_channel(env!("CARGO_PKG_VERSION")).is_some());
    }
}
