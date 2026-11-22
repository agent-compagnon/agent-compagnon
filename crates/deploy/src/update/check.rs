//! One check for an update: discovery, download into `update\<version>\`, every verification in the order of spec
//! 020, then the replacement. The first failure abandons the update and removes the downloaded files; the
//! [`ExeReplacer`] is called only once every check passed.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use url::Url;

use super::discover::{Channel, ListingError, choose, parse_listing};
use super::fetch::{Fetch, FetchError, Limits};
use super::keys::KeyRing;
use super::manifest::{MANIFEST_MAX, Manifest, ManifestError};
use super::replace::{ExeReplacer, ReplaceError, VerifiedExe};
use crate::install::EXE_NAME;
use crate::version::{Installed, Version, compare, is_newer, version_in};

/// Name of the manifest next to the exe.
pub const SIG_NAME: &str = "mcp-proxy.exe.sig";
/// Larger exes are not downloaded.
pub const EXE_DOWNLOAD_MAX: u64 = 256 * 1024 * 1024;
const LISTING_MAX: u64 = 1024 * 1024;
const SMALL_TIMEOUT: Duration = Duration::from_secs(30);
const EXE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Where the versions are published: `[update]` of the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub base_url: String,
    pub repository: String,
    pub path: String,
}

impl Source {
    fn url(&self, segments: &[&str]) -> Result<Url, UpdateError> {
        let base = self.base_url.trim_end_matches('/');
        Url::parse(&format!("{base}/{}", segments.join("/"))).map_err(|_| UpdateError::Address)
    }

    /// `{base_url}/api/storage/{repository}/{path}`.
    pub fn listing_url(&self) -> Result<Url, UpdateError> {
        self.url(&["api", "storage", &self.repository, &self.path])
    }

    /// `{base_url}/{repository}/{path}/{version}/{name}`.
    pub fn file_url(&self, version: &Version, name: &str) -> Result<Url, UpdateError> {
        self.url(&[&self.repository, &self.path, &version.to_string(), name])
    }
}

/// A check that failed: a French text without any path, URL or content of an answer, for the journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// No key embedded: the updater never replaces anything.
    NoKey,
    /// The installed exe has no readable version mark: it is never replaced (spec 019).
    InstalledUnknown,
    Address,
    Listing(FetchError),
    ListingFormat(ListingError),
    Download {
        file: &'static str,
        cause: FetchError,
    },
    /// Writing `update\<version>\` failed.
    Disk(std::io::ErrorKind),
    /// 1. The manifest is not well formed.
    Manifest(ManifestError),
    /// 2. `key-id` names none of the embedded keys.
    UnknownKey,
    /// 3. The Ed25519 signature does not hold (strict verification).
    Signature,
    /// 4. The version of the manifest is not the name of its folder.
    VersionMismatch {
        folder: Version,
        manifest: Version,
    },
    /// 5. Not strictly above the running and the installed versions: no going back.
    NotNewer {
        version: Version,
        floor: Version,
    },
    /// 6. The SHA-256 of the downloaded exe is not `sha256`.
    Digest,
    /// 7. The version mark of the downloaded exe is not `version`.
    Mark,
    Replace(ReplaceError),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoKey => f.write_str("aucune clé de mise à jour embarquée"),
            Self::InstalledUnknown => f.write_str("version de l'exe installé illisible : il n'est pas remplacé"),
            Self::Address => f.write_str("adresse d'Artifactory invalide"),
            Self::Listing(cause) => write!(f, "liste des versions illisible ({cause})"),
            Self::ListingFormat(cause) => write!(f, "{cause}"),
            Self::Download { file, cause } => write!(f, "téléchargement de {file} impossible ({cause})"),
            Self::Disk(kind) => write!(f, "dossier des téléchargements inutilisable ({kind})"),
            Self::Manifest(cause) => write!(f, "{cause}"),
            Self::UnknownKey => f.write_str("manifeste signé par une clé inconnue"),
            Self::Signature => f.write_str("signature du manifeste invalide"),
            Self::VersionMismatch { folder, manifest } => {
                write!(f, "le manifeste du dossier {folder} annonce la version {manifest}")
            }
            Self::NotNewer { version, floor } => {
                write!(f, "version {version} refusée : pas plus récente que {floor}")
            }
            Self::Digest => f.write_str("empreinte SHA-256 de l'exe différente de celle du manifeste"),
            Self::Mark => f.write_str("marque de version de l'exe différente de celle du manifeste"),
            Self::Replace(cause) => write!(f, "remplacement de l'exe : {cause}"),
        }
    }
}

/// An update put in place, effective at the next start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReady {
    pub version: Version,
    pub critical: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// Nothing above the floor on this channel.
    UpToDate,
    Ready(UpdateReady),
}

/// Checks 1 to 5, on the manifest alone, before the exe is downloaded.
pub fn verify_manifest(
    bytes: &[u8],
    keys: &KeyRing,
    folder: &Version,
    floor: &Version,
) -> Result<Manifest, UpdateError> {
    let manifest = Manifest::parse(bytes).map_err(UpdateError::Manifest)?;
    verify_signature(&manifest, keys)?;
    if manifest.version != *folder {
        return Err(UpdateError::VersionMismatch {
            folder: folder.clone(),
            manifest: manifest.version.clone(),
        });
    }
    if !is_newer(&manifest.version, floor) {
        return Err(UpdateError::NotNewer {
            version: manifest.version.clone(),
            floor: floor.clone(),
        });
    }
    Ok(manifest)
}

/// Checks 2 and 3.
pub fn verify_signature(manifest: &Manifest, keys: &KeyRing) -> Result<(), UpdateError> {
    let mut candidates = keys.find(manifest.key_id).peekable();
    if candidates.peek().is_none() {
        return Err(UpdateError::UnknownKey);
    }
    if candidates.any(|key| key.verify_strict(&manifest.signed, &manifest.signature).is_ok()) {
        Ok(())
    } else {
        Err(UpdateError::Signature)
    }
}

/// Checks 6 and 7, on the downloaded exe.
pub fn verify_exe(manifest: &Manifest, exe: &[u8]) -> Result<[u8; 32], UpdateError> {
    let digest: [u8; 32] = Sha256::digest(exe).into();
    if digest != manifest.sha256 {
        return Err(UpdateError::Digest);
    }
    if version_in(exe) != Installed::Known(manifest.version.clone()) {
        return Err(UpdateError::Mark);
    }
    Ok(digest)
}

/// What the updater is built from.
pub struct UpdaterDeps {
    pub source: Source,
    pub fetch: Arc<dyn Fetch>,
    pub replacer: Arc<dyn ExeReplacer>,
    pub keys: KeyRing,
    /// `%LOCALAPPDATA%\MCP Proxy\update`.
    pub downloads: PathBuf,
    /// The version of the running exe.
    pub current: Version,
}

pub struct Updater {
    deps: UpdaterDeps,
}

impl fmt::Debug for Updater {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Updater")
            .field("current", &self.deps.current.to_string())
            .field("keys", &self.deps.keys)
            .finish_non_exhaustive()
    }
}

/// The folder of one download, removed whole unless [`Download::keep`] is called.
struct Download {
    dir: PathBuf,
}

impl Download {
    fn create(root: &Path, version: &Version) -> Result<Self, UpdateError> {
        let dir = root.join(version.to_string());
        let disk = |e: std::io::Error| UpdateError::Disk(e.kind());
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(disk)?;
        }
        std::fs::create_dir_all(&dir).map_err(disk)?;
        Ok(Self { dir })
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, UpdateError> {
        let path = self.dir.join(name);
        std::fs::write(&path, bytes).map_err(|e| UpdateError::Disk(e.kind()))?;
        Ok(path)
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.dir)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(kind = %error.kind(), "mise à jour : fichiers téléchargés non supprimés");
        }
    }
}

impl Updater {
    pub fn new(deps: UpdaterDeps) -> Self {
        Self { deps }
    }

    pub fn current(&self) -> &Version {
        &self.deps.current
    }

    /// Without any key, no update can ever be accepted.
    pub fn has_keys(&self) -> bool {
        !self.deps.keys.is_empty()
    }

    /// Nothing at or below the floor is ever installed: the running version, the version already put in place by
    /// an earlier check (`ready`), and the installed one.
    fn floor(&self, ready: Option<&Version>) -> Result<Version, UpdateError> {
        let installed = match self.deps.replacer.installed() {
            Installed::Known(version) => version,
            Installed::Unknown => return Err(UpdateError::InstalledUnknown),
        };
        Ok([Some(&self.deps.current), ready, Some(&installed)]
            .into_iter()
            .flatten()
            .max_by(|a, b| compare(a, b))
            .cloned()
            .unwrap_or_else(|| self.deps.current.clone()))
    }

    /// One check. The downloaded files are removed whatever happens: on success, the exe is already copied into
    /// the installation.
    pub async fn check(&self, channel: Channel, ready: Option<&Version>) -> Result<CheckOutcome, UpdateError> {
        let deps = &self.deps;
        if deps.keys.is_empty() {
            return Err(UpdateError::NoKey);
        }
        let floor = self.floor(ready)?;
        let small = |max_bytes| Limits {
            max_bytes,
            timeout: SMALL_TIMEOUT,
        };
        let listing = deps
            .fetch
            .get(&deps.source.listing_url()?, small(LISTING_MAX))
            .await
            .map_err(UpdateError::Listing)?;
        let versions = parse_listing(&listing).map_err(UpdateError::ListingFormat)?;
        let Some(version) = choose(&versions, channel, &floor) else {
            return Ok(CheckOutcome::UpToDate);
        };
        tracing::info!(%version, "mise à jour : version candidate");

        let download = Download::create(&deps.downloads, &version)?;
        let sig = deps
            .fetch
            .get(&deps.source.file_url(&version, SIG_NAME)?, small(MANIFEST_MAX as u64))
            .await
            .map_err(|cause| UpdateError::Download {
                file: "la signature",
                cause,
            })?;
        download.write(SIG_NAME, &sig)?;
        let manifest = verify_manifest(&sig, &deps.keys, &version, &floor)?;

        let exe_limits = Limits {
            max_bytes: EXE_DOWNLOAD_MAX,
            timeout: EXE_TIMEOUT,
        };
        let exe = deps
            .fetch
            .get(&deps.source.file_url(&version, EXE_NAME)?, exe_limits)
            .await
            .map_err(|cause| UpdateError::Download { file: "l'exe", cause })?;
        let path = download.write(EXE_NAME, &exe)?;
        let sha256 = verify_exe(&manifest, &exe)?;
        drop(exe);

        let verified = VerifiedExe {
            path,
            version: manifest.version.clone(),
            sha256,
        };
        let replacer = Arc::clone(&deps.replacer);
        tokio::task::spawn_blocking(move || replacer.replace(&verified))
            .await
            .map_err(|_| UpdateError::Disk(std::io::ErrorKind::Interrupted))?
            .map_err(UpdateError::Replace)?;
        drop(download);
        Ok(CheckOutcome::Ready(UpdateReady {
            version: manifest.version,
            critical: manifest.critical,
        }))
    }
}
