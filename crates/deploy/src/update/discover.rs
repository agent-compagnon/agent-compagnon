//! Which version to take: the folders listed by the Storage API of Artifactory, `mcp-proxy/<semver>/`, without any
//! `latest.json` (the repository is a release one: nothing in it can change).

use std::fmt;

use serde_json::Value;

use crate::version::{Version, is_newer};

/// `UserSettings::beta_channel`, « Recevoir les préversions ».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Releases only: `1.4.0-rc.1` is ignored.
    Stable,
    /// Pre-releases too.
    Beta,
}

impl Channel {
    pub fn of(beta: bool) -> Self {
        if beta { Self::Beta } else { Self::Stable }
    }
}

/// The answer of `GET {base_url}/api/storage/{repository}/{path}` is not a folder listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListingError;

impl fmt::Display for ListingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("réponse de l'API Storage illisible")
    }
}

/// The versions among the `children` of a Storage API answer: folders whose name is a semver, written in its only
/// spelling. Files, other names and malformed entries are ignored; a missing `children` is an error.
pub fn parse_listing(body: &[u8]) -> Result<Vec<Version>, ListingError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| ListingError)?;
    let children = value.get("children").and_then(Value::as_array).ok_or(ListingError)?;
    Ok(children
        .iter()
        .filter(|child| child.get("folder").and_then(Value::as_bool) == Some(true))
        .filter_map(|child| child.get("uri").and_then(Value::as_str))
        .filter_map(|uri| uri.strip_prefix('/'))
        .filter_map(|name| Version::parse(name).ok().filter(|v| v.to_string() == name))
        .collect())
}

/// The highest version by semver precedence strictly above `floor`, pre-releases only on the beta channel.
pub fn choose(versions: &[Version], channel: Channel, floor: &Version) -> Option<Version> {
    versions
        .iter()
        .filter(|v| channel == Channel::Beta || v.pre.is_empty())
        .filter(|v| is_newer(v, floor))
        .max_by(|a, b| a.cmp_precedence(b))
        .cloned()
}
