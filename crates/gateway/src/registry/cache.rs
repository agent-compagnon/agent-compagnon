//! Local cache of the last live list of each instance.
//!
//! The content holds no secret, but it comes from servers: it goes through the output filter before being written,
//! and nothing is written if the filter fails. Sealing (spec 014) plugs in through [`CacheSeal`].

use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mcp_proxy_core::filter::{FilterError, OutputFilter};
use serde::{Deserialize, Serialize};

use super::model::{InstanceId, ToolDefinition};

/// Cache format understood by this build. Any other version is ignored.
pub const CACHE_SCHEMA_VERSION: u32 = 1;

/// Larger files are not caches written by this application.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// The cached list of one instance.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedCatalog {
    pub tools: Vec<ToolDefinition>,
}

/// Why a cache could not be read or written. French, quotes no content of the file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CacheError {
    #[error("cache du catalogue illisible : {0}")]
    Io(String),
    #[error("cache du catalogue invalide : {0}")]
    Invalid(String),
    #[error("cache du catalogue non enregistré : {0}")]
    Save(String),
    #[error("cache du catalogue non enregistré : {0}")]
    Filter(#[source] FilterError),
    #[error("cache du catalogue : scellement invalide ({0})")]
    Seal(String),
}

fn io_reason(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::PermissionDenied => "accès refusé".to_owned(),
        io::ErrorKind::NotFound => "fichier ou dossier introuvable".to_owned(),
        kind => format!("erreur d'entrée-sortie ({kind})"),
    }
}

pub trait CatalogCache: Send + Sync {
    /// `Ok(None)` when nothing is cached for `instance`. An error means the cache must be ignored.
    fn load(&self, instance: &InstanceId) -> Result<Option<CachedCatalog>, CacheError>;

    fn store(&self, instance: &InstanceId, catalog: &CachedCatalog) -> Result<(), CacheError>;
}

/// Integrity protection of the cache files, applied to the bytes written and read. The instance is given so that a
/// seal can bind a file to it (a file copied to another instance's name must not open).
pub trait CacheSeal: Send + Sync {
    fn seal(&self, instance: &InstanceId, plain: Vec<u8>) -> Result<Vec<u8>, CacheError>;

    fn open(&self, instance: &InstanceId, sealed: Vec<u8>) -> Result<Vec<u8>, CacheError>;
}

/// No protection, until spec 014.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSeal;

impl CacheSeal for NoSeal {
    fn seal(&self, _: &InstanceId, plain: Vec<u8>) -> Result<Vec<u8>, CacheError> {
        Ok(plain)
    }

    fn open(&self, _: &InstanceId, sealed: Vec<u8>) -> Result<Vec<u8>, CacheError> {
        Ok(sealed)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CacheFile {
    schema_version: u32,
    env: String,
    mcp: String,
    tools: Vec<ToolDefinition>,
}

/// One JSON file per instance, `<mcp>.<env>.json`, in an injected directory (`%LOCALAPPDATA%\MCP Proxy\cache\catalog`
/// in the application).
pub struct FileCatalogCache {
    dir: PathBuf,
    filter: Arc<OutputFilter>,
    seal: Box<dyn CacheSeal>,
}

impl std::fmt::Debug for FileCatalogCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileCatalogCache")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl FileCatalogCache {
    pub fn new(dir: impl Into<PathBuf>, filter: Arc<OutputFilter>) -> Self {
        Self::with_seal(dir, filter, Box::new(NoSeal))
    }

    pub fn with_seal(dir: impl Into<PathBuf>, filter: Arc<OutputFilter>, seal: Box<dyn CacheSeal>) -> Self {
        Self {
            dir: dir.into(),
            filter,
            seal,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Identifiers are `[a-z0-9-]`, so the name is a plain file name on every platform.
    pub fn path_for(&self, instance: &InstanceId) -> PathBuf {
        self.dir.join(format!("{}.{}.json", instance.mcp, instance.env))
    }
}

impl CatalogCache for FileCatalogCache {
    fn load(&self, instance: &InstanceId) -> Result<Option<CachedCatalog>, CacheError> {
        let file = match fs::File::open(self.path_for(instance)) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(CacheError::Io(io_reason(&e))),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| CacheError::Io(io_reason(&e)))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(CacheError::Invalid(format!(
                "fichier trop volumineux (plus de {MAX_FILE_BYTES} octets)"
            )));
        }
        let bytes = self.seal.open(instance, bytes)?;
        let file: CacheFile = serde_json::from_slice(&bytes).map_err(|e| {
            CacheError::Invalid(format!("JSON non conforme, ligne {}, colonne {}", e.line(), e.column()))
        })?;
        if file.schema_version != CACHE_SCHEMA_VERSION {
            return Err(CacheError::Invalid("version de format inconnue".to_owned()));
        }
        if file.env != instance.env.as_str() || file.mcp != instance.mcp.as_str() {
            return Err(CacheError::Invalid("fichier d'une autre instance".to_owned()));
        }
        Ok(Some(CachedCatalog { tools: file.tools }))
    }

    /// Filters, then writes a sibling temporary file and renames it. Nothing is written if the filter fails.
    fn store(&self, instance: &InstanceId, catalog: &CachedCatalog) -> Result<(), CacheError> {
        let file = CacheFile {
            schema_version: CACHE_SCHEMA_VERSION,
            env: instance.env.as_str().to_owned(),
            mcp: instance.mcp.as_str().to_owned(),
            tools: catalog.tools.clone(),
        };
        let mut value = serde_json::to_value(&file).map_err(|e| CacheError::Save(e.to_string()))?;
        self.filter.filter_json(&mut value).map_err(CacheError::Filter)?;
        let bytes = serde_json::to_vec_pretty(&value).map_err(|e| CacheError::Save(e.to_string()))?;
        let bytes = self.seal.seal(instance, bytes)?;

        let save_err = |e: io::Error| CacheError::Save(io_reason(&e));
        fs::create_dir_all(&self.dir).map_err(save_err)?;
        let path = self.path_for(instance);
        let mut tmp = path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let written = fs::File::create(&tmp).and_then(|mut f| {
            f.write_all(&bytes)?;
            f.sync_all()
        });
        if let Err(e) = written.and_then(|()| fs::rename(&tmp, &path)) {
            // Best effort: a leftover temporary file is harmless, it is never read.
            let _ = fs::remove_file(&tmp);
            return Err(save_err(e));
        }
        Ok(())
    }
}
