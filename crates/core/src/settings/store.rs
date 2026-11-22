//! Persistence of the user settings. Sealing (step 5d) will provide another `SettingsStore`.

use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use super::UserSettings;
use super::file::SettingsFile;

/// Larger files are not settings written by this application.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// Why the settings could not be read or written. French, quotes no content of the file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsError {
    #[error("réglages illisibles : {0}")]
    Io(String),
    #[error("réglages invalides : {0}")]
    Invalid(String),
    #[error("impossible d'enregistrer les réglages : {0}")]
    Save(String),
}

fn io_reason(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::PermissionDenied => "accès refusé".to_owned(),
        io::ErrorKind::NotFound => "fichier ou dossier introuvable".to_owned(),
        kind => format!("erreur d'entrée-sortie ({kind})"),
    }
}

/// Where the loaded settings come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsOrigin {
    /// Read from the store.
    Stored,
    /// Nothing stored yet (first launch): default settings.
    Missing,
    /// Unreadable or invalid: the strictest settings replace them and the user must be told.
    Reset(SettingsError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSettings {
    pub settings: UserSettings,
    pub origin: SettingsOrigin,
}

impl LoadedSettings {
    fn strictest(origin: SettingsOrigin) -> Self {
        Self {
            settings: UserSettings::default(),
            origin,
        }
    }
}

pub trait SettingsStore {
    /// Never fails: anything else than valid stored settings gives [`UserSettings::default`], the strictest.
    fn load(&self) -> LoadedSettings;

    fn save(&self, settings: &UserSettings) -> Result<(), SettingsError>;
}

/// Plain JSON file at an injected path.
#[derive(Debug, Clone)]
pub struct JsonFileStore {
    path: PathBuf,
}

impl JsonFileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<Option<Vec<u8>>, SettingsError> {
        let file = match fs::File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(SettingsError::Io(io_reason(&e))),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| SettingsError::Io(io_reason(&e)))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(SettingsError::Invalid(format!(
                "fichier trop volumineux (plus de {MAX_FILE_BYTES} octets)"
            )));
        }
        Ok(Some(bytes))
    }
}

pub(super) fn parse(bytes: &[u8]) -> Result<UserSettings, SettingsError> {
    let file: SettingsFile = serde_json::from_slice(bytes).map_err(|e| {
        // The serde message may quote values of the file: only its position is kept.
        SettingsError::Invalid(format!("JSON non conforme, ligne {}, colonne {}", e.line(), e.column()))
    })?;
    file.into_settings().map_err(SettingsError::Invalid)
}

pub(super) fn render(settings: &UserSettings) -> Result<Vec<u8>, SettingsError> {
    serde_json::to_vec_pretty(&SettingsFile::from_settings(settings)).map_err(|e| SettingsError::Save(e.to_string()))
}

impl SettingsStore for JsonFileStore {
    fn load(&self) -> LoadedSettings {
        let bytes = match self.read() {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return LoadedSettings::strictest(SettingsOrigin::Missing),
            Err(e) => return LoadedSettings::strictest(SettingsOrigin::Reset(e)),
        };
        match parse(&bytes) {
            Ok(settings) => LoadedSettings {
                settings,
                origin: SettingsOrigin::Stored,
            },
            Err(e) => LoadedSettings::strictest(SettingsOrigin::Reset(e)),
        }
    }

    /// Writes a sibling temporary file then renames it, so a crash never leaves half-written settings.
    fn save(&self, settings: &UserSettings) -> Result<(), SettingsError> {
        let bytes = render(settings)?;
        let save_err = |e: io::Error| SettingsError::Save(io_reason(&e));
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(save_err)?;
        }
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let written = fs::File::create(&tmp).and_then(|mut f| {
            f.write_all(&bytes)?;
            f.sync_all()
        });
        if let Err(e) = written.and_then(|()| fs::rename(&tmp, &self.path)) {
            // Best effort: a leftover temporary file is harmless, it is never read.
            let _ = fs::remove_file(&tmp);
            return Err(save_err(e));
        }
        Ok(())
    }
}
