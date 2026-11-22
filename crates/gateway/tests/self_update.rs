//! Applying the updates of MCP Proxy (spec 020) on a gateway started as the application starts it, against
//! `FakeArtifactory`, with injected time: every pause is released by the test, none is waited for.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::pin::pin;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use mcp_proxy_auth::UnavailableAuthUi;
use mcp_proxy_core::config::{EnvId, McpId, ToolName};
use mcp_proxy_core::confirm::ConfirmationRequest;
use mcp_proxy_core::secrets::{SecretStore, platform_key_wrapper};
use mcp_proxy_core::session::{SessionInfo, SessionRegistry, SessionState};
use mcp_proxy_core::trace::{JournalFilter, SystemWallClock};
use mcp_proxy_deploy::fake::FakeReplacer;
use mcp_proxy_deploy::update::{
    AnonymousClient, ExeReplacer, KeyRing, Manifest, Source, UpdateReady, Updater, UpdaterDeps,
};
use mcp_proxy_deploy::version::{Version, mark_prefix};
use mcp_proxy_gateway::interface::InterfaceError;
use mcp_proxy_gateway::notify::{Information, UnavailablePresenter, information_toast};
use mcp_proxy_gateway::runtime::{DataDirs, Gateway, Logging, Persistence, RuntimeDeps};
use mcp_proxy_gateway::self_update::{
    Activity, ActivityProbe, ActivitySignal, CRITICAL_GRACE, Pause, Restarter, SelfUpdateDeps, restart_when_idle,
    update_information, wait_until_idle,
};
use mcp_proxy_gateway::view::UpdateReadyView;
use mcp_proxy_testkit::{DEV_SIGNING_SEED, FakeArtifactory, Served};
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, broadcast, watch};

const REPO: &str = "generic-release-local";
const PATH: &str = "mcp-proxy";

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "mcp-proxy-self-update-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Pauses that end when the test says so. Each pause asked is recorded, in order.
#[derive(Default)]
struct StepPause {
    asked: watch::Sender<Vec<Duration>>,
    released: watch::Sender<usize>,
}

impl StepPause {
    /// Waits until `n` pauses were asked for, and gives them.
    async fn asked(&self, n: usize) -> Vec<Duration> {
        let mut asked = self.asked.subscribe();
        asked.wait_for(|a| a.len() >= n).await.unwrap().clone()
    }

    /// Ends every pause asked so far, and the next ones up to `n` in all.
    fn release(&self, n: usize) {
        self.released.send_modify(|r| *r = (*r).max(n));
    }
}

#[async_trait]
impl Pause for StepPause {
    async fn sleep(&self, duration: Duration) {
        let mut index = 0;
        self.asked.send_modify(|a| {
            index = a.len();
            a.push(duration);
        });
        let mut released = self.released.subscribe();
        let _ = released.wait_for(|r| *r > index).await;
    }
}

#[derive(Default)]
struct CountingRestarter {
    restarts: AtomicUsize,
    done: Notify,
}

impl Restarter for CountingRestarter {
    fn restart(&self) {
        self.restarts.fetch_add(1, Ordering::SeqCst);
        self.done.notify_waiters();
        self.done.notify_one();
    }
}

impl CountingRestarter {
    fn count(&self) -> usize {
        self.restarts.load(Ordering::SeqCst)
    }
}

fn exe(version: &str) -> Vec<u8> {
    let mut bytes = b"MZ an exe of MCP Proxy ".to_vec();
    bytes.extend_from_slice(mark_prefix());
    bytes.extend_from_slice(version.as_bytes());
    bytes.push(0x01);
    bytes
}

fn publish(art: &FakeArtifactory, version: &str, critical: bool) {
    let bytes = exe(version);
    let key = SigningKey::from_bytes(&DEV_SIGNING_SEED);
    let sig = Manifest::sign(&v(version), Sha256::digest(&bytes).into(), critical, &key).render();
    art.publish(version, bytes, sig.into_bytes());
}

struct World {
    _dir: TempDir,
    gateway: Arc<Gateway>,
    art: FakeArtifactory,
    replacer: Arc<FakeReplacer>,
    pause: Arc<StepPause>,
    restarter: Arc<CountingRestarter>,
}

impl World {
    async fn start(current: &str) -> Self {
        let dir = TempDir::new();
        let dirs = DataDirs::under(&dir.0);
        let config = Arc::new(mcp_proxy_gateway::modules::embedded_config().unwrap());
        let gateway = Arc::new(
            Gateway::start(RuntimeDeps {
                config,
                persistence: Persistence::sealed(&dirs, platform_key_wrapper()),
                dirs,
                store: SecretStore::for_platform().unwrap(),
                auth_ui: Arc::new(UnavailableAuthUi),
                presenter: Arc::new(UnavailablePresenter),
                clock: Arc::new(SystemWallClock),
                logging: Logging::Local,
                utc_offset: 0,
                system: "système de test".to_owned(),
            })
            .await
            .unwrap(),
        );
        let art = FakeArtifactory::start(REPO, PATH).await.unwrap();
        Self {
            replacer: Arc::new(FakeReplacer::new(v(current))),
            pause: Arc::default(),
            restarter: Arc::default(),
            gateway,
            art,
            _dir: dir,
        }
    }

    fn deps(&self, current: &str, keys: KeyRing) -> SelfUpdateDeps {
        let downloads = DataDirs::under(&self._dir.0).update;
        SelfUpdateDeps {
            updater: Updater::new(UpdaterDeps {
                source: Source {
                    base_url: self.art.base_url(),
                    repository: REPO.to_owned(),
                    path: PATH.to_owned(),
                },
                fetch: Arc::new(AnonymousClient::new().unwrap()),
                replacer: Arc::clone(&self.replacer) as Arc<dyn ExeReplacer>,
                keys,
                downloads,
                current: v(current),
            }),
            interval: Duration::from_secs(6 * 3600),
            pause: Arc::clone(&self.pause) as Arc<dyn Pause>,
            restarter: Arc::clone(&self.restarter) as Arc<dyn Restarter>,
        }
    }

    fn listing_hits(&self) -> usize {
        let path = format!("/artifactory/api/storage/{REPO}/{PATH}");
        self.art.requests().iter().filter(|r| r.path == path).count()
    }

    fn journal(&self) -> Vec<String> {
        self.gateway
            .journal()
            .snapshot(&JournalFilter::default())
            .into_iter()
            .map(|e| e.summary)
            .collect()
    }
}

fn dev_keys() -> KeyRing {
    KeyRing::new(&[SigningKey::from_bytes(&DEV_SIGNING_SEED).verifying_key().to_bytes()]).unwrap()
}

// Exigence : EX-020-16
#[tokio::test]
async fn checks_run_a_minute_after_start_then_at_each_interval() {
    let world = World::start("1.3.0").await;
    publish(&world.art, "1.4.0-rc.1", false);
    let _task = world
        .gateway
        .start_self_update(world.deps("1.3.0", dev_keys()))
        .unwrap();

    assert_eq!(world.pause.asked(1).await, [Duration::from_secs(60)]);
    assert_eq!(world.listing_hits(), 0, "nothing before the first minute");
    world.pause.release(1);
    assert_eq!(world.pause.asked(2).await[1], Duration::from_secs(6 * 3600));
    assert_eq!(world.listing_hits(), 1);
    assert_eq!(
        world.gateway.update_ready(),
        None,
        "stable channel: the pre-release is ignored"
    );

    world.pause.release(2);
    world.pause.asked(3).await;
    assert_eq!(world.listing_hits(), 2);

    // « Recevoir les préversions »: a check at once, without waiting for the interval.
    world.gateway.set_beta_channel(true).await.unwrap();
    assert!(world.gateway.settings().beta_channel);
    let asked = world.pause.asked(4).await;
    assert_eq!(asked[3], Duration::from_secs(6 * 3600));
    assert_eq!(world.listing_hits(), 3);
    assert_eq!(
        world.gateway.update_ready(),
        Some(UpdateReady {
            version: v("1.4.0-rc.1"),
            critical: false
        })
    );
    assert_eq!(world.replacer.calls().len(), 1);
}

// Exigence : EX-020-16
#[tokio::test]
async fn a_failed_check_only_shows_in_the_journal() {
    let world = World::start("1.3.0").await;
    world.art.serve_listing(Served::Status(503));
    let before = world.gateway.snapshot().await;
    let _task = world
        .gateway
        .start_self_update(world.deps("1.3.0", dev_keys()))
        .unwrap();
    world.pause.release(1);
    world.pause.asked(2).await;

    let journal = world.journal();
    let failure = journal
        .iter()
        .find(|s| s.starts_with("vérification des mises à jour en échec"))
        .unwrap();
    assert_eq!(
        failure,
        "vérification des mises à jour en échec : liste des versions illisible (réponse HTTP 503)"
    );
    for line in &journal {
        assert!(!line.contains("127.0.0.1") && !line.contains("/artifactory"), "{line}");
    }
    let after = world.gateway.snapshot().await;
    assert_eq!(after.alerts, before.alerts, "no banner");
    assert_eq!(after.status, before.status);
    assert_eq!(after.update, None);
}

// Exigence : EX-020-16
#[tokio::test]
async fn without_a_key_no_check_is_ever_made() {
    let world = World::start("1.3.0").await;
    publish(&world.art, "1.4.0", false);
    assert!(
        world
            .gateway
            .start_self_update(world.deps("1.3.0", KeyRing::default()))
            .is_none()
    );
    assert!(world.art.requests().is_empty());
    assert!(
        world
            .journal()
            .iter()
            .any(|s| s == "mises à jour automatiques désactivées : aucune clé de mise à jour dans cet exe")
    );
}

// Exigence : EX-020-17
#[tokio::test]
async fn an_update_in_place_is_in_the_snapshot_and_offered_for_a_restart() {
    let world = World::start("1.3.0").await;
    publish(&world.art, "1.4.0", false);
    assert_eq!(world.gateway.restart_for_update(), Err(InterfaceError::NoUpdateReady));
    let mut changes = world.gateway.update_changes();
    let _task = world
        .gateway
        .start_self_update(world.deps("1.3.0", dev_keys()))
        .unwrap();
    assert_eq!(
        world.gateway.restart_for_update(),
        Err(InterfaceError::NoUpdateReady),
        "nothing in place yet"
    );
    world.pause.release(1);
    let ready = changes.wait_for(Option::is_some).await.unwrap().clone().unwrap();
    assert_eq!(
        ready,
        UpdateReady {
            version: v("1.4.0"),
            critical: false
        }
    );

    let snapshot = world.gateway.snapshot().await;
    assert_eq!(
        snapshot.update,
        Some(UpdateReadyView {
            version: "1.4.0".to_owned(),
            critical: false
        })
    );
    let json = world.gateway.snapshot_json().await.unwrap();
    assert_eq!(
        json["update"],
        serde_json::json!({ "version": "1.4.0", "critical": false })
    );
    assert!(
        world
            .journal()
            .iter()
            .any(|s| s == "mise à jour 1.4.0 prête : elle sera utilisée au prochain démarrage de MCP Proxy")
    );
    let information = update_information(&ready);
    assert_eq!(
        information,
        Information::UpdateReady {
            version: "1.4.0".to_owned()
        }
    );
    assert_eq!(
        information_toast(1, &information).lines,
        ["MCP Proxy 1.4.0 sera utilisé au prochain démarrage."]
    );

    // Not restarted by itself: the user chooses when.
    world.pause.asked(2).await;
    assert_eq!(world.restarter.count(), 0);
    world.gateway.restart_for_update().unwrap();
    assert_eq!(world.restarter.count(), 1);
    assert!(
        world
            .journal()
            .iter()
            .any(|s| s == "redémarrage demandé pour passer à MCP Proxy 1.4.0")
    );
}

/// An activity set by the test.
struct FakeActivity {
    activity: Mutex<Activity>,
    changes: broadcast::Sender<()>,
    calls: watch::Sender<usize>,
}

impl FakeActivity {
    fn new(activity: Activity) -> Self {
        Self {
            activity: Mutex::new(activity),
            changes: broadcast::channel(4).0,
            calls: watch::Sender::new(activity.calls),
        }
    }

    fn set(&self, activity: Activity) {
        *self.activity.lock().unwrap() = activity;
        if self.calls.send_replace(activity.calls) == activity.calls {
            let _ = self.changes.send(());
        }
    }
}

impl ActivityProbe for FakeActivity {
    fn activity(&self) -> Activity {
        *self.activity.lock().unwrap()
    }

    fn signal(&self) -> ActivitySignal {
        ActivitySignal::new(self.changes.subscribe(), self.calls.subscribe())
    }
}

/// The future is waiting, and stays so: polled again without anything changing, it does not move.
async fn still_waiting<F: std::future::Future>(future: &mut std::pin::Pin<&mut F>) -> bool {
    for _ in 0..3 {
        if futures::poll!(future.as_mut()).is_ready() {
            return false;
        }
        tokio::task::yield_now().await;
    }
    true
}

// Exigence : EX-020-06
#[tokio::test]
async fn a_critical_restart_waits_for_every_call_login_and_confirmation() {
    let busy = Activity {
        calls: 1,
        logins: 1,
        confirmations: 1,
    };
    let probe = FakeActivity::new(busy);
    let pause = StepPause::default();
    let restarter = CountingRestarter::default();
    let mut restart = pin!(restart_when_idle(&probe, &pause, &restarter));

    // The grace period first, whatever the activity.
    assert!(still_waiting(&mut restart).await);
    assert_eq!(pause.asked(1).await, [CRITICAL_GRACE]);
    pause.release(1);
    assert!(still_waiting(&mut restart).await);

    // Then nothing while anything is in progress, each kind on its own.
    for remaining in [
        Activity { calls: 0, ..busy },
        Activity {
            calls: 0,
            logins: 0,
            confirmations: 1,
        },
        Activity {
            calls: 2,
            logins: 0,
            confirmations: 0,
        },
        Activity {
            calls: 0,
            logins: 1,
            confirmations: 0,
        },
    ] {
        probe.set(remaining);
        assert!(still_waiting(&mut restart).await, "{remaining:?}");
        assert_eq!(restarter.count(), 0, "{remaining:?}");
    }
    probe.set(Activity::default());
    restart.await;
    assert_eq!(restarter.count(), 1);

    // Idle already: no other wait than the grace period.
    let probe = FakeActivity::new(Activity::default());
    let pause = StepPause::default();
    pause.release(1);
    wait_until_idle(&probe).await;
    restart_when_idle(&probe, &pause, &restarter).await;
    assert_eq!(restarter.count(), 2);
}

// Exigence : EX-020-06
#[tokio::test]
async fn the_gateway_counts_what_a_critical_restart_waits_for() {
    let world = World::start("1.3.0").await;
    let gateway = &world.gateway;
    assert!(gateway.activity().is_idle());

    let call = gateway.calls_in_flight().start();
    let second = gateway.calls_in_flight().start();
    assert_eq!(gateway.activity().calls, 2);
    drop(call);
    assert_eq!(gateway.activity().calls, 1);
    drop(second);

    let request = ConfirmationRequest {
        env: EnvId::new("dev").unwrap(),
        mcp: McpId::new("kibana-logs").unwrap(),
        tool: ToolName::new("delete_index").unwrap(),
        path: "DEV / kibana-logs / delete_index".to_owned(),
        client: "Claude Desktop".to_owned(),
        arguments: serde_json::json!({}),
        explanation: "confirmation".to_owned(),
        deadline: Instant::now() + Duration::from_secs(30),
    };
    let waiting = gateway.confirmations().pending().register(request, false).unwrap();
    assert_eq!(
        gateway.activity(),
        Activity {
            calls: 0,
            logins: 0,
            confirmations: 1
        }
    );
    gateway.confirmations().pending().refuse_all();
    let _ = waiting.outcome().await;
    assert!(gateway.activity().is_idle());

    // A login asked for counts as well (state of the sessions, as the snapshot reads it).
    let sessions = SessionRegistry::from_config(gateway.config()).snapshot();
    let mut states: Vec<(SessionInfo, SessionState)> = sessions;
    states[0].1 = SessionState::LoginRequested {
        since: std::time::SystemTime::now(),
    };
    assert_eq!(mcp_proxy_gateway::self_update::logins_in(&states), 1);
    assert_eq!(gateway.logins_waiting(), 0);
}

// Exigence : EX-020-06
// Exigence : EX-020-17
#[tokio::test]
async fn a_critical_release_restarts_by_itself_once_nothing_is_in_progress() {
    let world = World::start("1.3.0").await;
    publish(&world.art, "1.4.0", true);
    let call = world.gateway.calls_in_flight().start();
    let _task = world
        .gateway
        .start_self_update(world.deps("1.3.0", dev_keys()))
        .unwrap();
    world.pause.release(1);
    // The check, then the grace period of the critical release.
    assert_eq!(world.pause.asked(2).await[1], CRITICAL_GRACE);
    let ready = world.gateway.update_ready().unwrap();
    assert!(ready.critical);
    assert_eq!(
        update_information(&ready),
        Information::CriticalUpdate {
            version: "1.4.0".to_owned()
        }
    );
    assert_eq!(
        information_toast(1, &update_information(&ready)).lines,
        ["MCP Proxy 1.4.0 redémarrera dès que plus aucun appel n'est en cours. Il faudra ensuite vous reconnecter."]
    );
    assert!(world.gateway.snapshot().await.update.unwrap().critical);

    let restarted = world.restarter.done.notified();
    world.pause.release(2);
    tokio::task::yield_now().await;
    assert_eq!(world.restarter.count(), 0, "a call is in progress");
    drop(call);
    restarted.await;
    assert_eq!(world.restarter.count(), 1);
    assert!(
        world
            .journal()
            .iter()
            .any(|s| s == "redémarrage automatique pour la mise à jour critique 1.4.0")
    );
}
