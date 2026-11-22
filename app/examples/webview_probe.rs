//! Sonde des fenêtres de connexion WebView2 (spec 012), à lancer sous Windows :
//!
//! ```text
//! cargo run -p mcp-proxy --example webview_probe
//! cargo run -p mcp-proxy --example webview_probe -- --profil ephemeral_disk   (contrôle : point 3 en ÉCHEC)
//! ```
//!
//! Elle ouvre les vraies fenêtres de l'appli (`mcp_proxy::login::WebviewAuthUi`) sur `FakeSso`, un site de login
//! local, et affiche une ligne par point : RÉUSSI ou ÉCHEC, avec la raison. Les fenêtres s'ouvrent et se ferment
//! seules ; ne pas les toucher. Aucun jeton n'est affiché. Code de sortie : 0 si tout est RÉUSSI, 1 sinon.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use mcp_proxy::login::{KEEPER, LoginState, WebviewAuthUi};
use mcp_proxy_auth::method::JwtHeaderMethod;
use mcp_proxy_auth::webview::{check_runtime_version, reset_profile_dir};
use mcp_proxy_auth::{AuthUi, LoginEvent, LoginWindow, LoginWindowSpec};
use mcp_proxy_core::config::{
    AuthConfig, Origin, Session, SessionId, SessionScope, UrlPattern, WebviewConfig, WebviewProfile,
};
use mcp_proxy_testkit::{FAKE_SSO_AUDIENCE, FAKE_SSO_COOKIE, FAKE_SSO_ISSUER, FakeSso, SsoToken};
use secrecy::{ExposeSecret as _, SecretString};
use tauri::{AppHandle, Manager, RunEvent};
use url::Url;

const WAIT: Duration = Duration::from_secs(20);

/// One line of the report, written as soon as it is known.
fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

struct Report {
    results: Mutex<Vec<bool>>,
}

impl Report {
    fn point(&self, number: u8, title: &str, outcome: Result<String, String>) {
        let ok = outcome.is_ok();
        match outcome {
            Ok(detail) if detail.is_empty() => say(&format!("{number}. RÉUSSI — {title}")),
            Ok(detail) => say(&format!("{number}. RÉUSSI — {title} ({detail})")),
            Err(reason) => say(&format!("{number}. ÉCHEC  — {title} : {reason}")),
        }
        self.results.lock().unwrap_or_else(PoisonError::into_inner).push(ok);
    }

    fn all_passed(&self) -> (usize, usize) {
        let results = self.results.lock().unwrap_or_else(PoisonError::into_inner);
        (results.iter().filter(|ok| **ok).count(), results.len())
    }
}

/// Options of the command line.
struct Options {
    profile: WebviewProfile,
    os_sso: bool,
    cookie_wait: Duration,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        profile: WebviewProfile::InMemory,
        os_sso: false,
        cookie_wait: Duration::from_secs(35),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--profil" => match args.next().as_deref() {
                Some("in_memory") => options.profile = WebviewProfile::InMemory,
                Some("ephemeral_disk") => options.profile = WebviewProfile::EphemeralDisk,
                _ => return Err("--profil attend in_memory ou ephemeral_disk".to_owned()),
            },
            "--sso-windows" => options.os_sso = true,
            "--attente-cookies" => {
                let secs = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or("--attente-cookies attend des secondes")?;
                options.cookie_wait = Duration::from_secs(secs);
            }
            other => return Err(format!("argument inconnu : {other}")),
        }
    }
    Ok(options)
}

/// The probe's own `get_snapshot`: a page that obtained this answer could reach the application's commands.
#[tauri::command]
fn get_snapshot() -> &'static str {
    "SONDE-COMMANDE-ATTEINTE"
}

fn main() -> ExitCode {
    let options = match options() {
        Ok(options) => options,
        Err(error) => {
            say(&format!("Sonde WebView2 : {error}"));
            return ExitCode::from(2);
        }
    };
    let Some(base) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        say("Sonde WebView2 : LOCALAPPDATA absent");
        return ExitCode::FAILURE;
    };
    // Its own folder: a running MCP Proxy keeps its profile.
    let data_dir = base.join("MCP Proxy").join("webview-probe");
    if let Err(error) = reset_profile_dir(&data_dir) {
        say(&format!(
            "Sonde WebView2 : dossier {} impossible à vider ({error})",
            data_dir.display()
        ));
        return ExitCode::FAILURE;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            say(&format!("Sonde WebView2 : runtime tokio impossible ({error})"));
            return ExitCode::FAILURE;
        }
    };
    tauri::async_runtime::set(runtime.handle().clone());

    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            mcp_proxy::login::deposit_login_header,
            mcp_proxy::login::submit_basic_credentials
        ])
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            say(&format!("Sonde WebView2 : Tauri n'a pas démarré ({error})"));
            return ExitCode::FAILURE;
        }
    };
    let config = WebviewConfig {
        profile: options.profile,
        os_primary_account_sso: options.os_sso,
    };
    let ui = Arc::new(WebviewAuthUi::new(app.handle(), &config, data_dir.clone(), Ok(())));
    let report = Arc::new(Report {
        results: Mutex::new(Vec::new()),
    });

    say("Sonde des fenêtres de connexion WebView2 — MCP Proxy (spec 012)");
    say(&format!(
        "Profil : {} · SSO du compte Windows : {} · dossier : {}",
        match options.profile {
            WebviewProfile::InMemory => "in_memory (InPrivate)",
            WebviewProfile::EphemeralDisk => "ephemeral_disk (contrôle : le point 3 doit être en ÉCHEC)",
        },
        if options.os_sso { "oui" } else { "non" },
        data_dir.display()
    ));
    say("Les fenêtres s'ouvrent et se ferment seules : ne pas les toucher.");

    let handle = app.handle().clone();
    let task_report = Arc::clone(&report);
    tauri::async_runtime::spawn(async move {
        probe(&handle, ui, &task_report, &data_dir, options.cookie_wait).await;
        handle.exit(0);
    });
    app.run_return(|_, event| {
        // No window between two points: only the probe ends the application.
        if let RunEvent::ExitRequested { api, code: None, .. } = event {
            api.prevent_exit();
        }
    });
    runtime.shutdown_background();

    let (passed, total) = report.all_passed();
    say(&format!("Bilan : {passed}/{total} RÉUSSI"));
    if passed == total && total == 8 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// A `jwt_header` session on `FakeSso`, starting on `path`.
fn spec(sso: &FakeSso, path: &str) -> Result<LoginWindowSpec, String> {
    let login_url = Url::parse(&sso.url(path)).map_err(|e| e.to_string())?;
    let prefix = Url::parse(&sso.api_prefix()).map_err(|e| e.to_string())?;
    let session = Session {
        id: SessionId::new("sonde").map_err(|e| e.to_string())?,
        label: "Sonde".to_owned(),
        scope: SessionScope::Global,
        auth: AuthConfig::JwtHeader {
            login_url: login_url.clone(),
            capture_url_pattern: UrlPattern { prefix, wildcard: true },
            capture_header: "Authorization".to_owned(),
            capture_scheme: "Bearer".to_owned(),
            expected_audience: Some(FAKE_SSO_AUDIENCE.to_owned()),
            expected_issuer: Some(FAKE_SSO_ISSUER.to_owned()),
            allowed_navigation: vec![sso.host().to_owned()],
            max_ttl_hours: 1,
        },
        allowed_origins: Origin::of(&login_url).into_iter().collect(),
    };
    JwtHeaderMethod::from_config(&session)
        .map(|m| m.window_spec())
        .ok_or_else(|| "session de sonde invalide".to_owned())
}

/// The first header captured by `window` within `WAIT`, as `(url, value)`; navigations are skipped.
async fn captured(window: &mut Box<dyn LoginWindow>) -> Result<(Url, SecretString), String> {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        match tokio::time::timeout_at(deadline, window.next_event()).await {
            Err(_) => return Err(format!("aucun en-tête déposé en {} s", WAIT.as_secs())),
            Ok(None | Some(LoginEvent::Closed)) => return Err("fenêtre fermée".to_owned()),
            Ok(Some(LoginEvent::Navigated { .. })) => {}
            Ok(Some(LoginEvent::HeaderCaptured { url, value, .. })) => return Ok((url, value)),
        }
    }
}

/// Waits until `window` has loaded a page whose path is `path`.
async fn loaded(window: &mut Box<dyn LoginWindow>, path: &str) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        match tokio::time::timeout_at(deadline, window.next_event()).await {
            Err(_) => return Err(format!("page {path} non chargée en {} s", WAIT.as_secs())),
            Ok(None | Some(LoginEvent::Closed)) => return Err("fenêtre fermée".to_owned()),
            Ok(Some(LoginEvent::Navigated { url })) if url.path() == path => return Ok(()),
            Ok(Some(_)) => {}
        }
    }
}

fn bearer(sso: &FakeSso, token: SsoToken) -> String {
    format!("Bearer {}", sso.token(token))
}

/// Files of `dir` (recursively) that contain `needle`, and how many could not be read (locked by WebView2).
fn files_containing(dir: &Path, needle: &[u8]) -> (Vec<PathBuf>, usize) {
    let mut found = Vec::new();
    let mut unreadable = 0;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            unreadable += 1;
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(path),
                Ok(kind) if kind.is_file() => match std::fs::read(&path) {
                    Ok(bytes) if bytes.windows(needle.len()).any(|w| w == needle) => found.push(path),
                    Ok(_) => {}
                    Err(_) => unreadable += 1,
                },
                _ => {}
            }
        }
    }
    (found, unreadable)
}

fn no_cookie_on_disk(dir: &Path) -> Result<String, String> {
    let (found, unreadable) = files_containing(dir, FAKE_SSO_COOKIE.as_bytes());
    let locked = if unreadable == 0 {
        String::new()
    } else {
        format!("{unreadable} fichier(s) verrouillé(s) non lu(s)")
    };
    if found.is_empty() {
        Ok(locked)
    } else {
        let names: Vec<String> = found.iter().map(|p| p.display().to_string()).collect();
        Err(format!("cookie {FAKE_SSO_COOKIE} trouvé dans {}", names.join(", ")))
    }
}

/// The open login window of the application (the keeper excepted), to read its URL.
fn open_login_url(app: &AppHandle) -> Option<Url> {
    app.webview_windows()
        .into_iter()
        .find(|(label, _)| label.starts_with("login-") && label != KEEPER)
        .and_then(|(_, window)| window.url().ok())
}

/// Points 1 (fetch), 2 and 3, on the window of `/app/fetch`.
async fn fetch_window(
    ui: &WebviewAuthUi,
    sso: &FakeSso,
    data_dir: &Path,
    cookie_wait: Duration,
    version: Option<&str>,
) -> (Result<(), String>, Result<String, String>, Result<String, String>) {
    let spec = match spec(sso, "/login/fetch") {
        Ok(spec) => spec,
        Err(error) => return (Err(error.clone()), Err(error.clone()), Err(error)),
    };
    let mut window = match ui.login_window(spec).await {
        Ok(window) => window,
        Err(error) => {
            let error = format!("fenêtre refusée : {error}");
            return (Err(error.clone()), Err(error.clone()), Err(error));
        }
    };
    let first = match captured(&mut window).await {
        Ok((url, value)) if value.expose_secret() == bearer(sso, SsoToken::Fetch) => {
            if url.as_str().starts_with(&sso.api_prefix()) {
                Ok(())
            } else {
                Err("URL déposée hors du motif".to_owned())
            }
        }
        Ok(_) => Err("valeur déposée différente de l'en-tête envoyé par fetch".to_owned()),
        Err(error) => Err(format!("fetch : {error}")),
    };
    let cookie = match Url::parse(&sso.url("/")) {
        Err(error) => Err(error.to_string()),
        Ok(site) => match window.cookies(&site).await {
            Err(error) => Err(error.to_string()),
            Ok(cookies) => {
                let expected = sso.session_cookie();
                match cookies.iter().find(|c| c.name == FAKE_SSO_COOKIE) {
                    None => Err(format!("cookie {FAKE_SSO_COOKIE} absent")),
                    Some(c) if Some(c.value.expose_secret()) == expected.as_deref() => Ok(if c.expires.is_none() {
                        "cookie de session".to_owned()
                    } else {
                        String::new()
                    }),
                    Some(_) => Err("valeur du cookie différente de celle posée".to_owned()),
                }
            }
        },
    };
    say(&format!(
        "   … attente de {} s, fenêtre ouverte (écriture différée des cookies par WebView2)",
        cookie_wait.as_secs()
    ));
    tokio::time::sleep(cookie_wait).await;
    let inprivate = match check_runtime_version(version) {
        Err(_) => Err("version du runtime refusée (< 101.0.1210.39 ou illisible)".to_owned()),
        Ok(_) if !ui.profile().incognito => Err(match no_cookie_on_disk(data_dir) {
            Ok(_) => "profil ephemeral_disk, pas d'InPrivate ; cookie non trouvé sur le disque : détecteur à revoir"
                .to_owned(),
            Err(found) => format!("profil ephemeral_disk, pas d'InPrivate ; {found} : détecteur vérifié"),
        }),
        Ok(_) => no_cookie_on_disk(data_dir),
    };
    window.close().await;
    (first, cookie, inprivate)
}

async fn probe(app: &AppHandle, ui: Arc<WebviewAuthUi>, report: &Report, data_dir: &Path, cookie_wait: Duration) {
    let sso = match FakeSso::start().await {
        Ok(sso) => sso,
        Err(error) => {
            say(&format!("FakeSso n'a pas démarré : {error}"));
            return;
        }
    };
    let version = tauri::webview_version().ok();
    say(&format!(
        "Runtime WebView2 : {}",
        version.as_deref().unwrap_or("illisible")
    ));

    // 1, 2, 3: fetch window, its HttpOnly cookie, InPrivate.
    let (first, cookie, inprivate) = fetch_window(&ui, &sso, data_dir, cookie_wait, version.as_deref()).await;

    // 1 (suite): XMLHttpRequest.
    let xhr = match spec(&sso, "/login/xhr") {
        Err(error) => Err(error),
        Ok(spec) => match ui.login_window(spec).await {
            Err(error) => Err(format!("fenêtre refusée : {error}")),
            Ok(mut window) => {
                let result = match captured(&mut window).await {
                    Ok((_, value)) if value.expose_secret() == bearer(&sso, SsoToken::Xhr) => Ok(()),
                    Ok(_) => Err("valeur déposée différente de l'en-tête envoyé par XMLHttpRequest".to_owned()),
                    Err(error) => Err(format!("XMLHttpRequest : {error}")),
                };
                window.close().await;
                result
            }
        },
    };
    report.point(
        1,
        "en-tête Authorization capturé par le script et déposé (fetch, XMLHttpRequest)",
        first.and(xhr).map(|()| String::new()),
    );
    report.point(2, "cookie HttpOnly lu", cookie);
    report.point(
        3,
        "InPrivate effectif (version du runtime, aucun cookie dans le dossier pendant la session)",
        inprivate,
    );

    // 4: a second window, while the keeper lives, is still logged in.
    let shared = match spec(&sso, "/whoami") {
        Err(error) => Err(error),
        Ok(spec) => match ui.login_window(spec).await {
            Err(error) => Err(format!("fenêtre refusée : {error}")),
            Ok(mut window) => {
                let result = loaded(&mut window, "/whoami")
                    .await
                    .and_then(|()| match sso.whoami().last() {
                        Some(true) => Ok(String::new()),
                        Some(false) => Err("la seconde fenêtre n'a pas envoyé le cookie de la session".to_owned()),
                        None => Err("page /whoami non demandée".to_owned()),
                    });
                window.close().await;
                result
            }
        },
    };
    report.point(
        4,
        "SSO partagé entre deux fenêtres successives (gardienne vivante)",
        shared,
    );

    // 5: leaving the allowed hosts.
    let blocked = match spec(&sso, "/login/escape") {
        Err(error) => Err(error),
        Ok(spec) => match ui.login_window(spec).await {
            Err(error) => Err(format!("fenêtre refusée : {error}")),
            Ok(mut window) => {
                let result = match loaded(&mut window, "/app/escape").await {
                    Err(error) => Err(error),
                    Ok(()) => {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        let current = open_login_url(app);
                        if sso.hits("/escaped") > 0 {
                            Err("la page hors liste a été demandée".to_owned())
                        } else if current.as_ref().and_then(Url::host_str) != Some(sso.host()) {
                            Err("la fenêtre a quitté l'hôte autorisé".to_owned())
                        } else {
                            Ok(String::new())
                        }
                    }
                };
                window.close().await;
                result
            }
        },
    };
    report.point(5, "navigation hors de la liste bloquée", blocked);

    // 7: from the remote page, every command but the deposit is refused.
    let commands = match spec(&sso, "/login/commands") {
        Err(error) => Err(error),
        Ok(spec) => match ui.login_window(spec).await {
            Err(error) => Err(format!("fenêtre refusée : {error}")),
            Ok(window) => {
                let deadline = tokio::time::Instant::now() + WAIT;
                while !sso.reports().iter().any(|(name, _)| name == "done" || name == "ipc")
                    && tokio::time::Instant::now() < deadline
                {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                window.close().await;
                let reports = sso.reports();
                let expected: BTreeSet<&str> =
                    ["get_snapshot", "submit_basic_credentials", "plugin:event|listen"].into();
                if reports.iter().any(|(name, _)| name == "ipc") {
                    Err("IPC de Tauri absente de la page : le dépôt ne peut pas fonctionner non plus".to_owned())
                } else if !reports.iter().any(|(name, _)| name == "done") {
                    Err("la page n'a pas terminé ses essais".to_owned())
                } else {
                    let allowed: Vec<&str> = reports
                        .iter()
                        .filter(|(name, result)| expected.contains(name.as_str()) && result != "refused")
                        .map(|(name, _)| name.as_str())
                        .collect();
                    let tried = reports
                        .iter()
                        .filter(|(name, _)| expected.contains(name.as_str()))
                        .count();
                    if !allowed.is_empty() {
                        Err(format!("commandes atteintes : {}", allowed.join(", ")))
                    } else if tried != expected.len() {
                        Err("essais incomplets".to_owned())
                    } else {
                        Ok("get_snapshot, submit_basic_credentials, plugin:event|listen refusées".to_owned())
                    }
                }
            }
        },
    };

    // 8: forged deposits, and a deposit from another window, are not retained.
    let forged = match spec(&sso, "/login/forged") {
        Err(error) => Err(error),
        Ok(spec) => match ui.login_window(spec).await {
            Err(error) => Err(format!("fenêtre refusée : {error}")),
            Ok(mut window) => {
                let state = app.try_state::<Arc<LoginState>>();
                if let (Some(state), Ok(page), Ok(api)) = (
                    state.as_ref(),
                    Url::parse(&sso.url("/app/forged")),
                    Url::parse(&sso.url("/api/other-window")),
                ) {
                    // `login-0` is never given to a window.
                    let value = SecretString::from(bearer(&sso, SsoToken::Forged));
                    state.windows.deposit(
                        "login-0",
                        Some(&page),
                        Some(api.as_str()),
                        Some(value),
                        SystemTime::now(),
                    );
                }
                let result = match captured(&mut window).await {
                    Ok((_, value)) if value.expose_secret() == bearer(&sso, SsoToken::Final) => Ok(String::new()),
                    Ok(_) => Err("une valeur forgée ou non conforme a été retenue".to_owned()),
                    Err(error) => Err(format!("jeton final non retenu : {error}")),
                };
                window.close().await;
                result
            }
        },
    };

    // 6: once every window and the keeper are gone, nothing persistent on disk.
    if let Some(keeper) = app.get_webview_window(KEEPER) {
        let _ = keeper.destroy();
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    report.point(
        6,
        "dossier de données sans cookie persistant après fermeture",
        no_cookie_on_disk(data_dir),
    );
    report.point(
        7,
        "depuis la page distante, toute autre commande que le dépôt est refusée",
        commands,
    );
    report.point(
        8,
        "dépôt d'une autre fenêtre ou valeur non conforme (URL, schéma, JWT expiré, audience) non retenu",
        forged,
    );
}
