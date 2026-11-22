//! `FakeArtifactory`: a generic repository of Artifactory for the updater (spec 020), with the Storage API
//! (`/artifactory/api/storage/{repo}/{path}`) and the files (`/artifactory/{repo}/{path}/{version}/{name}`). Every
//! request is recorded with its headers. Any file can be replaced by a trap: other bytes, a body cut short of its
//! announced length, a redirect, a status.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::Response;
use http::header::{CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use http::{HeaderValue, StatusCode};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// The seed of the development signing key (spec 020). Its public half is `crates/deploy/keys/dev.pub`: a key whose
/// private half is in the repository, for tests only, refused by every build of the application.
pub const DEV_SIGNING_SEED: [u8; 32] = *b"mcp-proxy dev key -- tests only!";

/// What a path answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Served {
    Bytes(Vec<u8>),
    /// `Content-Length` of the whole, then the connection breaks after `sent` bytes.
    Truncated {
        bytes: Vec<u8>,
        sent: usize,
    },
    /// `302` towards this location.
    Redirect(String),
    Status(u16),
}

/// A request received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactoryRequest {
    pub path: String,
    /// Names in lower case, with their values.
    pub headers: Vec<(String, String)>,
}

#[derive(Default)]
struct Repo {
    /// Children of the listing: name and whether it is a folder.
    children: Vec<(String, bool)>,
    /// Replaces the whole listing answer.
    listing: Option<Served>,
    /// Paths below `/artifactory/{repo}/{path}/`.
    files: BTreeMap<String, Served>,
    requests: Vec<ArtifactoryRequest>,
}

struct Site {
    repository: String,
    path: String,
    repo: Mutex<Repo>,
}

fn lock(site: &Site) -> MutexGuard<'_, Repo> {
    site.repo.lock().unwrap_or_else(PoisonError::into_inner)
}

fn status(code: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = code;
    response
}

fn serve(served: Served) -> Response {
    match served {
        Served::Bytes(bytes) => Response::new(Body::from(bytes)),
        Served::Truncated { bytes, sent } => {
            let whole = bytes.len();
            let first = bytes.into_iter().take(sent).collect::<Vec<u8>>();
            let chunks: Vec<Result<Vec<u8>, std::io::Error>> =
                vec![Ok(first), Err(std::io::Error::other("connexion coupée"))];
            let mut response = Response::new(Body::from_stream(futures::stream::iter(chunks)));
            response.headers_mut().insert(CONTENT_LENGTH, HeaderValue::from(whole));
            response
        }
        Served::Redirect(location) => {
            let mut response = status(StatusCode::FOUND);
            if let Ok(value) = HeaderValue::from_str(&location) {
                response.headers_mut().insert(LOCATION, value);
            }
            response
        }
        Served::Status(code) => status(StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)),
    }
}

async fn handle(State(site): State<Arc<Site>>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let headers = request
        .headers()
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), value.to_str().unwrap_or_default().to_owned()))
        .collect();
    let mut repo = lock(&site);
    repo.requests.push(ArtifactoryRequest {
        path: path.clone(),
        headers,
    });
    let listing = format!("/artifactory/api/storage/{}/{}", site.repository, site.path);
    if path == listing {
        if let Some(served) = repo.listing.clone() {
            return serve(served);
        }
        let children: Vec<Value> = repo
            .children
            .iter()
            .map(|(name, folder)| json!({ "uri": format!("/{name}"), "folder": folder }))
            .collect();
        let body = json!({
            "repo": site.repository,
            "path": format!("/{}", site.path),
            "created": "2026-11-01T09:00:00.000Z",
            "children": children,
            "uri": format!("http://artifactory{listing}"),
        });
        let mut response = Response::new(Body::from(body.to_string()));
        response.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/vnd.org.jfrog.artifactory.storage.FolderInfo+json"),
        );
        return response;
    }
    let prefix = format!("/artifactory/{}/{}/", site.repository, site.path);
    match path.strip_prefix(&prefix).and_then(|file| repo.files.get(file)) {
        Some(served) => serve(served.clone()),
        None => status(StatusCode::NOT_FOUND),
    }
}

/// The fake repository, on `127.0.0.1` with a free port, plain `http` (allowed towards the loopback only). Stops
/// when dropped.
pub struct FakeArtifactory {
    addr: SocketAddr,
    site: Arc<Site>,
    ct: CancellationToken,
}

impl FakeArtifactory {
    /// An empty repository `repository`, versions under `path`.
    pub async fn start(repository: &str, path: &str) -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let site = Arc::new(Site {
            repository: repository.to_owned(),
            path: path.to_owned(),
            repo: Mutex::default(),
        });
        let router = Router::new().fallback(handle).with_state(Arc::clone(&site));
        let ct = CancellationToken::new();
        let shutdown = ct.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await;
        });
        Ok(Self { addr, site, ct })
    }

    /// `http://127.0.0.1:<port>`.
    pub fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// `http://127.0.0.1:<port>/artifactory`, the `update.base_url`.
    pub fn base_url(&self) -> String {
        format!("http://{}/artifactory", self.addr)
    }

    /// The same server under another origin (`localhost` instead of `127.0.0.1`), for redirects.
    pub fn other_origin(&self, path: &str) -> String {
        format!("http://localhost:{}{path}", self.addr.port())
    }

    /// The path of a file as requested: `/artifactory/{repo}/{path}/{version}/{name}`.
    pub fn file_path(&self, version: &str, name: &str) -> String {
        format!(
            "/artifactory/{}/{}/{version}/{name}",
            self.site.repository, self.site.path
        )
    }

    /// The folder `version` with its exe and its manifest.
    pub fn publish(&self, version: &str, exe: Vec<u8>, sig: Vec<u8>) {
        self.add_child(version, true);
        self.serve(version, "mcp-proxy.exe", Served::Bytes(exe));
        self.serve(version, "mcp-proxy.exe.sig", Served::Bytes(sig));
    }

    /// One more entry in the listing: a folder or a file.
    pub fn add_child(&self, name: &str, folder: bool) {
        let mut repo = lock(&self.site);
        if !repo.children.iter().any(|(n, _)| n == name) {
            repo.children.push((name.to_owned(), folder));
        }
    }

    /// What `{version}/{name}` answers from now on.
    pub fn serve(&self, version: &str, name: &str, served: Served) {
        lock(&self.site).files.insert(format!("{version}/{name}"), served);
    }

    /// What the Storage API answers from now on, instead of the listing.
    pub fn serve_listing(&self, served: Served) {
        lock(&self.site).listing = Some(served);
    }

    pub fn requests(&self) -> Vec<ArtifactoryRequest> {
        lock(&self.site).requests.clone()
    }

    /// Requests received for `{version}/{name}`.
    pub fn hits(&self, version: &str, name: &str) -> usize {
        let path = self.file_path(version, name);
        lock(&self.site).requests.iter().filter(|r| r.path == path).count()
    }
}

impl Drop for FakeArtifactory {
    fn drop(&mut self) {
        self.ct.cancel();
    }
}
