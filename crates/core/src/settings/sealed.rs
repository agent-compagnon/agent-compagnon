//! The settings file with its seal (spec 014): what replaces the plain JSON file in the application.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::UserSettings;
use super::store::{
    LoadedSettings, SettingsError, SettingsOrigin, SettingsStore, io_reason, read_file, render, write_atomic,
};
use crate::sealed::{SETTINGS_PURPOSE, Sealer};

/// `settings.json` sealed by a [`Sealer`]. Anything but a file whose seal holds gives the strictest settings:
/// modified outside of the application, without a seal (previous version), or a key that cannot open it.
#[derive(Debug, Clone)]
pub struct SealedSettingsStore {
    path: PathBuf,
    sealer: Arc<Sealer>,
}

impl SealedSettingsStore {
    pub fn new(path: impl Into<PathBuf>, sealer: Arc<Sealer>) -> Self {
        Self {
            path: path.into(),
            sealer,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl SettingsStore for SealedSettingsStore {
    fn load(&self) -> LoadedSettings {
        let sealed = match read_file(&self.path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return LoadedSettings::strictest(SettingsOrigin::Missing),
            Err(e) => return LoadedSettings::strictest(SettingsOrigin::Reset(e)),
        };
        match self.sealer.unseal(SETTINGS_PURPOSE, &sealed) {
            Ok(content) => LoadedSettings::parsed(&content),
            Err(e) => LoadedSettings::strictest(SettingsOrigin::Reset(SettingsError::Seal(e))),
        }
    }

    fn save(&self, settings: &UserSettings) -> Result<(), SettingsError> {
        let content = render(settings)?;
        let sealed = self
            .sealer
            .seal(SETTINGS_PURPOSE, &content)
            .map_err(|e| SettingsError::Save(e.to_string()))?;
        write_atomic(&self.path, &sealed).map_err(|e| SettingsError::Save(io_reason(&e)))
    }
}
