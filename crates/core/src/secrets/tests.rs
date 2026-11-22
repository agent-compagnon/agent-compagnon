use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proptest::prelude::*;
use reqwest::Method;
use reqwest::header::{AUTHORIZATION, COOKIE, HeaderValue};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

use super::key::{ClearKey, key_probe};
use super::sealed::{associated_data, drop_probe};
use super::test_support::{LyingWrapper, TestWrapper};
use super::*;
use crate::config::{Config, Origin, SessionId};

const SENTINEL: &str = "S3NT1NEL-6f1c2b9e-do-not-print";

/// Clock that only moves when told to.
struct ManualClock(Mutex<Instant>);

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Instant::now())))
    }

    fn advance(&self, by: Duration) {
        let mut now = self.0.lock().unwrap();
        *now += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

const GRACE: Duration = Duration::from_secs(130);

/// A store with `GRACE` and a manual clock.
fn store_with_clock() -> (SecretStore, Arc<ManualClock>) {
    let clock = ManualClock::new();
    let store = SecretStore::new(Box::new(TestWrapper::default()))
        .unwrap()
        .with_variant_grace(GRACE)
        .with_clock(clock.clone());
    (store, clock)
}

fn variant_strings(store: &SecretStore) -> Vec<String> {
    let mut variants = store
        .with_variants(|v| {
            v.iter()
                .map(|n| String::from_utf8_lossy(n).into_owned())
                .collect::<Vec<_>>()
        })
        .unwrap();
    variants.sort();
    variants
}

fn sid(id: &str) -> SessionId {
    SessionId::new(id).unwrap()
}

fn store() -> SecretStore {
    SecretStore::new(Box::new(TestWrapper::default())).unwrap()
}

/// A store whose wrapper can be watched and broken after creation.
fn watched_store() -> (SecretStore, TestWrapper) {
    let wrapper = TestWrapper::default();
    (SecretStore::new(Box::new(wrapper.clone())).unwrap(), wrapper)
}

/// A store holding a bearer token for `sso-corp`, whose wrapper fails from now on.
fn broken_store(token: &str) -> SecretStore {
    let (store, wrapper) = watched_store();
    store.put(&sid("sso-corp"), bearer(token)).unwrap();
    wrapper.break_now();
    store
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn bearer(token: &str) -> Credential {
    Credential::Bearer { token: secret(token) }
}

fn basic(login: &str, password: &str) -> Credential {
    Credential::Basic {
        login: login.to_owned(),
        password: secret(password),
    }
}

fn cookies(pairs: &[(&str, &str)]) -> Credential {
    Credential::Cookies {
        pairs: pairs.iter().map(|(n, v)| ((*n).to_owned(), secret(v))).collect(),
    }
}

fn token_of(store: &SecretStore, session: &SessionId) -> Result<String, SecretError> {
    store.with_exposed(session, |c| match c {
        ExposedCredential::Bearer { token } => token.to_owned(),
        other => panic!("unexpected credential {other:?}"),
    })
}

// --- Envelope protection --------------------------------------------------------------------------------------------

// Exigence : EX-002-11
#[cfg(not(windows))]
#[test]
fn insecure_dev_wrapper_round_trip() {
    let wrapper = InsecureDevKeyWrapper::new();
    let key: Vec<u8> = (0u8..32).collect();
    let wrapped = wrapper.wrap(&key).unwrap();
    assert_eq!(wrapper.unwrap(&wrapped).unwrap().as_slice(), key.as_slice());
}

// Exigence : EX-002-11
#[test]
fn sealed_secret_round_trip() {
    let key = ClearKey::generate().unwrap();
    let aad = associated_data(&sid("sso-corp"), "bearer");
    for len in [0usize, 1, 15, 16, 17, 31, 32, 100, MAX_SECRET_LEN] {
        let value: Vec<u8> = (0..len).map(|i| (i % 251) as u8 + 1).collect();
        let sealed = SealedSecret::seal(&key, &aad, &value).unwrap();
        assert_eq!(sealed.len(), len);
        assert_eq!(sealed.is_empty(), len == 0);
        assert_eq!(sealed.unseal(&key, &aad).unwrap().as_bytes(), value.as_slice());
        assert_eq!(
            sealed.unseal(&key, &aad).unwrap().as_bytes(),
            value.as_slice(),
            "unseal works on a copy"
        );
    }
}

// Exigence : EX-002-11
#[test]
fn each_sealing_uses_a_fresh_nonce() {
    let key = ClearKey::generate().unwrap();
    let aad = associated_data(&sid("sso-corp"), "bearer");
    let a = SealedSecret::seal(&key, &aad, SENTINEL.as_bytes()).unwrap();
    let b = SealedSecret::seal(&key, &aad, SENTINEL.as_bytes()).unwrap();
    assert_ne!(format!("{:?}", a.debug_bytes()), format!("{:?}", b.debug_bytes()));
    assert!(
        !a.debug_bytes()
            .windows(SENTINEL.len())
            .any(|w| w == SENTINEL.as_bytes())
    );
}

#[test]
fn sealed_secret_refuses_oversized_values() {
    let key = ClearKey::generate().unwrap();
    let value = vec![b'a'; MAX_SECRET_LEN + 1];
    assert_eq!(
        SealedSecret::seal(&key, b"aad", &value).unwrap_err(),
        SecretError::Protection(ProtectionFailure::TooLarge)
    );
}

// Exigence : EX-002-11
#[test]
fn platform_store_round_trip() {
    let store = SecretStore::for_platform().unwrap();
    let s = sid("sso-corp");
    store.put(&s, bearer(SENTINEL)).unwrap();
    assert_eq!(token_of(&store, &s).unwrap(), SENTINEL);
}

// Exigence : EX-002-12
#[test]
fn tampered_secret_is_detected() {
    let key = ClearKey::generate().unwrap();
    let aad = associated_data(&sid("sso-corp"), "bearer");
    let mut sealed = SealedSecret::seal(&key, &aad, SENTINEL.as_bytes()).unwrap();
    sealed.tamper(3);
    assert_eq!(sealed.unseal(&key, &aad).err(), Some(SecretError::Corrupted));
}

// Exigence : EX-002-12
#[test]
fn tampered_credential_or_variant_fails_closed_in_the_store() {
    let store = store();
    let s = sid("sso-corp");
    store.put(&s, bearer(SENTINEL)).unwrap();
    store.tamper_for_test(&s, false);
    let mut called = false;
    assert_eq!(store.with_exposed(&s, |_| called = true), Err(SecretError::Corrupted));
    assert!(!called);

    store.put(&s, bearer(SENTINEL)).unwrap();
    store.tamper_for_test(&s, true);
    assert_eq!(store.with_variants(|_| called = true), Err(SecretError::Corrupted));
    assert!(!called);
}

// Exigence : EX-002-13
#[test]
fn sealed_value_is_bound_to_its_session_and_slot() {
    let key = ClearKey::generate().unwrap();
    let corp = associated_data(&sid("sso-corp"), "bearer");
    let sealed = SealedSecret::seal(&key, &corp, SENTINEL.as_bytes()).unwrap();
    for other in [
        associated_data(&sid("sso-prod"), "bearer"),
        associated_data(&sid("sso-corp"), "variant"),
        associated_data(&sid("sso-corp"), "cookie:SID"),
    ] {
        assert_eq!(sealed.unseal(&key, &other).err(), Some(SecretError::Corrupted));
    }
    let other_key = ClearKey::generate().unwrap();
    assert_eq!(sealed.unseal(&other_key, &corp).err(), Some(SecretError::Corrupted));
}

// Exigence : EX-002-13
#[test]
fn secret_moved_to_another_session_does_not_open() {
    let store = store();
    let (corp, prod) = (sid("sso-corp"), sid("sso-prod"));
    store.put(&corp, cookies(&[("SID", SENTINEL)])).unwrap();
    store.move_entry_for_test(&corp, &prod);
    assert!(store.has(&prod));
    let mut called = false;
    assert_eq!(
        store.with_exposed(&prod, |_| called = true),
        Err(SecretError::Corrupted)
    );
    assert!(!called);
    assert_eq!(store.with_variants(|_| ()), Err(SecretError::Corrupted));
}

// Exigence : EX-002-14
#[test]
fn key_is_unwrapped_once_per_use_and_wiped() {
    let (store, wrapper) = watched_store();
    let (corp, prod) = (sid("sso-corp"), sid("sso-prod"));
    let base = wrapper.unwraps();
    store
        .put(&corp, cookies(&[("SID", "cookie-value-1"), ("XSRF", "cookie-value-2")]))
        .unwrap();
    store.put(&prod, bearer(SENTINEL)).unwrap();
    assert_eq!(wrapper.unwraps(), base + 2, "one unwrap per put");
    key_probe::take();

    store
        .with_exposed(&corp, |_| {
            let dropped = key_probe::take();
            assert_eq!(dropped.len(), 1, "key wiped before the closure runs");
            assert!(
                dropped
                    .iter()
                    .all(|k| k.len() == SESSION_KEY_LEN && k.iter().all(|b| *b == 0))
            );
        })
        .unwrap();
    assert_eq!(wrapper.unwraps(), base + 3, "one unwrap for every cookie");

    store.wipe(&corp);
    let count = store.with_variants(|v| v.len()).unwrap();
    assert!(count >= 3, "variants of both sessions, active and retired");
    assert_eq!(wrapper.unwraps(), base + 4, "one unwrap for every variant");
    let dropped = key_probe::take();
    assert_eq!(dropped.len(), 1);
    assert!(dropped[0].iter().all(|b| *b == 0));

    // No secret: no unwrap at all.
    store.with_exposed(&corp, |_| ()).unwrap_err();
    assert_eq!(wrapper.unwraps(), base + 4);
}

// Exigence : EX-002-15
#[test]
fn unwrap_failure_fails_closed() {
    let store = broken_store(SENTINEL);
    let s = sid("sso-corp");
    let mut called = false;
    let error = store.with_exposed(&s, |_| called = true).unwrap_err();
    assert!(
        matches!(error, SecretError::Protection(ProtectionFailure::Os(_))),
        "{error:?}"
    );
    assert!(store.with_variants(|_| called = true).is_err());
    assert!(!called);
    assert!(store.put(&sid("sso-prod"), bearer("other-token")).is_err());
    assert!(!store.has(&sid("sso-prod")));
    assert!(store.has(&s), "the previous secret stays in place");
}

// Exigence : EX-002-15
#[test]
fn store_creation_checks_the_wrapper() {
    let wrapper = TestWrapper::default();
    wrapper.break_now();
    let error = SecretStore::new(Box::new(wrapper)).err();
    assert_eq!(
        error,
        Some(SecretError::KeyProtectionUnavailable(ProtectionFailure::Os(-1)))
    );
    let error = SecretStore::new(Box::new(LyingWrapper)).err();
    assert_eq!(
        error,
        Some(SecretError::KeyProtectionUnavailable(ProtectionFailure::RoundTrip))
    );
    assert!(
        error
            .unwrap()
            .to_string()
            .starts_with("protection des secrets indisponible")
    );
}

// Exigence : EX-002-04
#[test]
fn sealed_buffer_is_zeroed_on_drop() {
    let key = ClearKey::generate().unwrap();
    let sealed = SealedSecret::seal(&key, b"aad", SENTINEL.as_bytes()).unwrap();
    drop_probe::take();
    drop(sealed);
    let last = drop_probe::take().unwrap();
    assert_eq!(last.len(), SENTINEL.len() + 16);
    assert!(last.iter().all(|b| *b == 0));
}

// Exigence : EX-002-04
#[test]
fn replaced_and_wiped_secrets_are_zeroed() {
    let store = store();
    let s = sid("sso-corp");
    store.put(&s, bearer(SENTINEL)).unwrap();
    drop_probe::take();
    store.put(&s, bearer("second-token-value")).unwrap();
    assert!(drop_probe::take().unwrap().iter().all(|b| *b == 0));
    store.wipe(&s);
    assert!(drop_probe::take().unwrap().iter().all(|b| *b == 0));
}

// --- No secret in any output ---------------------------------------------------------------------------------------

/// Every Debug/Display output reachable with `value` as a secret.
fn outputs_with_secret(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let store = store();
    let s = sid("sso-corp");
    let credentials = [
        bearer(value),
        cookies(&[("SID", value), ("XSRF", "other-cookie-value")]),
        basic("jdoe", value),
    ];
    for credential in credentials {
        out.push(format!("{credential:?}"));
        store.put(&s, credential).unwrap();
        out.push(format!("{store:?}"));
        store
            .with_exposed(&s, |exposed| {
                out.push(format!("{exposed:?}"));
                if let ExposedCredential::Cookies { pairs } = exposed {
                    out.extend(pairs.iter().map(|c| format!("{c:?}")));
                }
            })
            .unwrap();
        store
            .with_variants(|v| out.push(format!("{} variants", v.len())))
            .unwrap();
    }
    let key = ClearKey::generate().unwrap();
    let sealed = SealedSecret::seal(&key, b"aad", value.as_bytes()).unwrap();
    out.push(format!("{sealed:?}"));

    let mut errors = Vec::new();
    for invalid in [
        bearer(&format!("{value}\r\nX-Injected: 1")),
        cookies(&[("SID", &format!("{value};extra=1"))]),
        basic("jd:oe", value),
    ] {
        out.push(format!("{invalid:?}"));
        errors.push(store.put(&s, invalid).unwrap_err());
    }
    store.wipe(&s);
    errors.push(store.with_exposed(&s, |_| ()).unwrap_err());
    let broken = broken_store(value);
    errors.push(broken.with_exposed(&s, |_| ()).unwrap_err());
    errors.push(broken.with_variants(|_| ()).unwrap_err());
    let guard = guard_for(&s, &["https://mcp.example.internal"]);
    let mut url = Url::parse("https://mcp.example.internal/").unwrap();
    url.set_username("user").unwrap();
    url.set_password(Some(value)).unwrap();
    url.query_pairs_mut().append_pair("q", value);
    errors.push(guard.authorize(&s, &url).unwrap_err());
    for error in errors {
        out.push(format!("{error}"));
        out.push(format!("{error:?}"));
    }
    out
}

// Exigence : EX-002-02
#[test]
fn no_secret_in_debug_or_display() {
    for output in outputs_with_secret(SENTINEL) {
        assert!(!output.contains(SENTINEL), "secret leaked in {output}");
        assert!(!output.contains("6f1c2b9e"), "part of the secret leaked in {output}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    // Exigence : EX-002-02
    #[test]
    fn no_secret_in_any_output(value in "[!-:<-~]{12,64}") {
        for output in outputs_with_secret(&value) {
            prop_assert!(!output.contains(&value), "secret leaked in {}", output);
        }
    }
}

// Exigence : EX-002-02
#[test]
fn authenticated_client_debug_has_no_secret() {
    let s = sid("sso-corp");
    let store = Arc::new(store());
    store.put(&s, bearer(SENTINEL)).unwrap();
    let guard = Arc::new(guard_for(&s, &["https://mcp.example.internal"]));
    let client = AuthenticatedHttpClient::new(s, store, guard, Duration::from_secs(5)).unwrap();
    assert!(!format!("{client:?}").contains(SENTINEL));
}

// --- Store -----------------------------------------------------------------------------------------------------------

// Exigence : EX-002-03
#[test]
fn put_replaces_the_session_secret() {
    let store = store();
    let s = sid("sso-corp");
    assert!(!store.has(&s));
    store.put(&s, bearer("first-token")).unwrap();
    store.put(&s, bearer("second-token")).unwrap();
    assert!(store.has(&s));
    assert_eq!(token_of(&store, &s).unwrap(), "second-token");
}

// Exigence : EX-002-03
#[test]
fn wipe_empties_only_that_session() {
    let store = store();
    let (a, b) = (sid("sso-corp"), sid("sso-prod"));
    store.put(&a, bearer("token-a")).unwrap();
    store.put(&b, bearer("token-b")).unwrap();
    assert!(store.wipe(&a));
    assert!(!store.wipe(&a));
    assert!(!store.has(&a));
    assert_eq!(token_of(&store, &b).unwrap(), "token-b");
    store.wipe_all();
    assert!(!store.has(&b));
    assert_eq!(store.with_exposed(&b, |_| ()), Err(SecretError::NoSecret(b.clone())));
}

// Exigence : EX-010-12
#[test]
fn wiping_a_version_keeps_a_newer_secret() {
    let store = store();
    let (a, b) = (sid("sso-corp"), sid("sso-prod"));
    let first = store.put(&a, bearer("token-1")).unwrap();
    let other = store.put(&b, bearer("token-b")).unwrap();
    assert_ne!(first, other);
    assert_eq!(store.version(&a), Some(first));
    let (token, seen) = store
        .with_exposed_versioned(&a, |c| match c {
            ExposedCredential::Bearer { token } => token.to_owned(),
            _ => String::new(),
        })
        .unwrap();
    assert_eq!((token.as_str(), seen), ("token-1", first));

    let second = store.put(&a, bearer("token-2")).unwrap();
    assert!(second > first);
    assert!(!store.wipe_version(&a, first), "a replaced secret is not wiped again");
    assert_eq!(token_of(&store, &a).unwrap(), "token-2");
    assert!(!store.wipe_version(&a, other), "a version of another session");
    assert!(store.wipe_version(&a, second));
    assert!(!store.has(&a));
    assert!(!store.wipe_version(&a, second));
    assert!(store.has(&b));
}

// Exigence : EX-002-03
#[test]
fn with_exposed_on_empty_session_fails_without_detail() {
    let store = store();
    let s = sid("sso-corp");
    store.put(&s, bearer(SENTINEL)).unwrap();
    store.wipe(&s);
    let error = store.with_exposed(&s, |_| ()).unwrap_err();
    assert_eq!(error, SecretError::NoSecret(s));
    assert_eq!(
        error.to_string(),
        "aucun secret pour la session « sso-corp » : une connexion est nécessaire"
    );
}

#[test]
fn sessions_are_isolated() {
    let store = store();
    store.put(&sid("sso-corp"), bearer("corp-token")).unwrap();
    assert_eq!(
        store.with_exposed(&sid("sso-prod"), |_| ()),
        Err(SecretError::NoSecret(sid("sso-prod")))
    );
}

#[test]
fn every_credential_kind_is_exposed() {
    let store = store();
    let s = sid("sso-corp");
    store
        .put(&s, cookies(&[("SID", "abc123"), ("XSRF", "def456")]))
        .unwrap();
    let seen = store
        .with_exposed(&s, |c| match c {
            ExposedCredential::Cookies { pairs } => pairs
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect::<Vec<_>>(),
            other => panic!("unexpected credential {other:?}"),
        })
        .unwrap();
    assert_eq!(seen, ["SID=abc123", "XSRF=def456"]);

    store.put(&s, basic("jdoe", "pa:ss word")).unwrap();
    let seen = store
        .with_exposed(&s, |c| match c {
            ExposedCredential::Basic { login, password } => format!("{login}|{password}"),
            other => panic!("unexpected credential {other:?}"),
        })
        .unwrap();
    assert_eq!(seen, "jdoe|pa:ss word");
}

#[test]
fn closures_may_call_back_into_the_store() {
    let store = store();
    let s = sid("sso-corp");
    store.put(&s, bearer("token")).unwrap();
    store.with_exposed(&s, |_| store.wipe(&s)).unwrap();
    assert!(!store.has(&s));
}

#[test]
fn variants_cover_the_raw_values_of_every_active_secret() {
    let store = store();
    store.put(&sid("sso-corp"), bearer("bearer-value")).unwrap();
    store.put(&sid("sso-tools"), basic("jdoe", "basic-password")).unwrap();
    store
        .put(
            &sid("sso-prod"),
            cookies(&[("A", "cookie-1"), ("B", "cookie-2"), ("C", "cookie-1")]),
        )
        .unwrap();
    let variants = variant_strings(&store);
    for raw in ["basic-password", "bearer-value", "cookie-1", "cookie-2"] {
        assert_eq!(variants.iter().filter(|v| *v == raw).count(), 1, "{raw}");
    }
    assert!(!variants.iter().any(|v| v == "jdoe"));
}

// --- Retired variants ------------------------------------------------------------------------------------------------

// Exigence : EX-002-10
#[test]
fn wiped_secret_variants_stay_known_during_the_grace_period() {
    let (store, clock) = store_with_clock();
    let s = sid("sso-corp");
    store.put(&s, bearer("wiped-token-value")).unwrap();
    assert!(store.wipe(&s));
    assert!(!store.has(&s));
    assert_eq!(store.with_exposed(&s, |_| ()), Err(SecretError::NoSecret(s.clone())));
    assert!(variant_strings(&store).contains(&"wiped-token-value".to_owned()));

    clock.advance(GRACE - Duration::from_secs(1));
    assert!(variant_strings(&store).contains(&"wiped-token-value".to_owned()));
    clock.advance(Duration::from_secs(1));
    assert!(variant_strings(&store).is_empty());
}

// Exigence : EX-002-10
#[test]
fn wipe_all_retires_every_secret() {
    let (store, clock) = store_with_clock();
    store.put(&sid("sso-corp"), bearer("token-a-value")).unwrap();
    store.put(&sid("sso-prod"), basic("jdoe", "password-b")).unwrap();
    store.wipe_all();
    assert!(!store.has(&sid("sso-corp")) && !store.has(&sid("sso-prod")));
    let variants = variant_strings(&store);
    assert!(variants.contains(&"token-a-value".to_owned()));
    assert!(variants.contains(&"password-b".to_owned()));
    clock.advance(GRACE);
    assert!(variant_strings(&store).is_empty());
}

// Exigence : EX-002-10
#[test]
fn put_keeps_the_retired_variants_of_the_previous_secret() {
    let (store, clock) = store_with_clock();
    let s = sid("sso-corp");
    store.put(&s, bearer("first-token-value")).unwrap();
    clock.advance(Duration::from_secs(60));
    store.wipe(&s);
    store.put(&s, bearer("second-token-value")).unwrap();
    clock.advance(Duration::from_secs(60));
    store.put(&s, bearer("third-token-value")).unwrap();
    assert_eq!(token_of(&store, &s).unwrap(), "third-token-value");
    let variants = variant_strings(&store);
    for raw in ["first-token-value", "second-token-value", "third-token-value"] {
        assert!(variants.contains(&raw.to_owned()), "{raw}");
    }

    // The first secret was retired at t = 60 s, the second one at t = 120 s.
    clock.advance(GRACE - Duration::from_secs(60));
    let variants = variant_strings(&store);
    assert!(!variants.contains(&"first-token-value".to_owned()));
    assert!(variants.contains(&"second-token-value".to_owned()));
    clock.advance(Duration::from_secs(60));
    assert_eq!(
        variant_strings(&store)
            .iter()
            .filter(|v| v.contains("token-value"))
            .count(),
        1
    );
    assert!(variant_strings(&store).contains(&"third-token-value".to_owned()));
}

// Exigence : EX-002-10
#[test]
fn expired_variants_are_zeroed() {
    let (store, clock) = store_with_clock();
    let s = sid("sso-corp");
    store.put(&s, bearer(SENTINEL)).unwrap();
    store.wipe(&s);
    clock.advance(GRACE);
    drop_probe::take();
    store.with_variants(|v| assert!(v.is_empty())).unwrap();
    assert!(drop_probe::take().unwrap().iter().all(|b| *b == 0));
}

#[test]
fn retired_variants_fail_closed_too() {
    let (store, wrapper) = watched_store();
    let s = sid("sso-corp");
    store.put(&s, bearer("token")).unwrap();
    store.wipe(&s);
    wrapper.break_now();
    let mut called = false;
    assert!(store.with_variants(|_| called = true).is_err());
    assert!(!called);
}

#[test]
fn invalid_credentials_are_refused_and_keep_the_previous_secret() {
    let store = store();
    let s = sid("sso-corp");
    store.put(&s, bearer("good-token")).unwrap();
    for invalid in [
        bearer(""),
        bearer("tok\r\nX-Injected: 1"),
        bearer(&"a".repeat(MAX_SECRET_LEN + 1)),
        cookies(&[]),
        cookies(&[("bad name", "v")]),
        cookies(&[("SID", "")]),
        cookies(&[("SID", "a;b")]),
        cookies(&[("SID", "a"), ("SID", "b")]),
        basic("", "pw"),
        basic("jd:oe", "pw"),
        basic("jdoe", ""),
        basic("jdoe", "p\nw"),
    ] {
        let description = format!("{invalid:?}");
        assert!(
            matches!(store.put(&s, invalid), Err(SecretError::InvalidCredential(_))),
            "{description}"
        );
    }
    assert_eq!(token_of(&store, &s).unwrap(), "good-token");
}

// --- Origin guard ----------------------------------------------------------------------------------------------------

fn guard_for(session: &SessionId, origins: &[&str]) -> OriginGuard {
    let origins: BTreeSet<Origin> = origins
        .iter()
        .map(|o| Origin::of(&Url::parse(o).unwrap()).unwrap())
        .collect();
    OriginGuard::new([(session.clone(), origins)])
}

// Exigence : EX-002-05
#[test]
fn authorize_accepts_only_declared_origins() {
    let s = sid("sso-corp");
    let guard = guard_for(&s, &["https://mcp.example.internal", "http://127.0.0.1:8080"]);
    let ok = |u: &str| guard.authorize(&s, &Url::parse(u).unwrap());

    assert_eq!(ok("https://mcp.example.internal/mcp?x=1"), Ok(()));
    assert_eq!(ok("https://mcp.example.internal:443/other"), Ok(()));
    assert_eq!(ok("http://127.0.0.1:8080/"), Ok(()));

    assert!(matches!(
        ok("https://evil.example.internal/mcp"),
        Err(SecretError::OriginNotAllowed { .. })
    ));
    assert!(matches!(
        ok("https://mcp.example.internal.evil/mcp"),
        Err(SecretError::OriginNotAllowed { .. })
    ));
    assert!(matches!(
        ok("https://mcp.example.internal:8443/mcp"),
        Err(SecretError::OriginNotAllowed { .. })
    ));
    assert!(matches!(
        ok("http://127.0.0.1:8081/"),
        Err(SecretError::OriginNotAllowed { .. })
    ));
    assert!(matches!(
        ok("http://mcp.example.internal/mcp"),
        Err(SecretError::InsecureScheme { .. })
    ));
    assert!(matches!(
        ok("ftp://mcp.example.internal/"),
        Err(SecretError::InsecureScheme { .. })
    ));
    assert!(matches!(
        ok("https://user@mcp.example.internal/"),
        Err(SecretError::CredentialsInUrl { .. })
    ));
    assert_eq!(
        guard.authorize(&sid("sso-prod"), &Url::parse("https://mcp.example.internal/").unwrap()),
        Err(SecretError::UnknownSession(sid("sso-prod")))
    );
}

// Exigence : EX-002-05
#[test]
fn origins_are_per_session() {
    let (a, b) = (sid("sso-corp"), sid("sso-prod"));
    let origin = |u: &str| Origin::of(&Url::parse(u).unwrap()).unwrap();
    let guard = OriginGuard::new([
        (a.clone(), BTreeSet::from([origin("https://a.example.internal")])),
        (b.clone(), BTreeSet::from([origin("https://b.example.internal")])),
    ]);
    let url = Url::parse("https://b.example.internal/mcp").unwrap();
    assert!(guard.authorize(&a, &url).is_err());
    assert!(guard.authorize(&b, &url).is_ok());
}

#[test]
fn guard_from_embedded_config() {
    let config = Config::embedded().unwrap();
    let guard = OriginGuard::from_config(&config);
    let tools = sid("sso-tools");
    let url = |u: &str| Url::parse(u).unwrap();
    // Local module `base_url` and Basic `probe_url`.
    assert_eq!(
        guard.authorize(&tools, &url("https://jira.example.internal/rest/api/2/search")),
        Ok(())
    );
    for session in &config.sessions {
        for origin in &session.allowed_origins {
            assert_eq!(guard.authorize(&session.id, &url(origin.as_str())), Ok(()));
        }
    }
    assert!(
        guard
            .authorize(&tools, &url("https://kibana.example.internal/"))
            .is_err()
    );
}

// --- Authenticated HTTP client ---------------------------------------------------------------------------------------

/// Minimal HTTP/1.1 server: records each request head and answers with a fixed response.
struct TestServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
}

impl TestServer {
    async fn start(response: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut head = Vec::new();
                let mut chunk = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&chunk[..n]),
                    }
                }
                recorded
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&head).into_owned());
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        Self { addr, requests }
    }

    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://{}{path}", self.addr)).unwrap()
    }

    fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn ok_response(extra_headers: &str) -> String {
    format!("HTTP/1.1 200 OK\r\n{extra_headers}Content-Length: 2\r\nConnection: close\r\n\r\nok")
}

/// Value of a request header, matched case-insensitively on the name.
fn header_value(request: &str, name: &str) -> Option<String> {
    request.lines().find_map(|line| {
        let (n, v) = line.split_once(':')?;
        n.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
    })
}

fn client_for(session: &SessionId, store: &Arc<SecretStore>, origins: &[&str]) -> AuthenticatedHttpClient {
    AuthenticatedHttpClient::new(
        session.clone(),
        Arc::clone(store),
        Arc::new(guard_for(session, origins)),
        Duration::from_secs(5),
    )
    .unwrap()
}

/// Asserts that nobody connected to `listener` within a short delay.
async fn assert_no_connection(listener: &TcpListener) {
    let accepted = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
    assert!(accepted.is_err(), "a connection was made");
}

// Exigence : EX-002-06
#[tokio::test]
async fn injects_each_credential_kind() {
    let server = TestServer::start(ok_response("")).await;
    let s = sid("sso-corp");
    let store = Arc::new(store());
    let client = client_for(&s, &store, &[&server.origin()]);

    store.put(&s, bearer(SENTINEL)).unwrap();
    let mut request = client.request(Method::GET, server.url("/bearer"));
    request
        .headers_mut()
        .insert(AUTHORIZATION, HeaderValue::from_static("Bearer caller-value"));
    request
        .headers_mut()
        .insert(COOKIE, HeaderValue::from_static("caller=1"));
    let response = client.execute(request).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "ok");

    store.put(&s, basic("jdoe", "pass word")).unwrap();
    client
        .execute(client.request(Method::GET, server.url("/basic")))
        .await
        .unwrap();

    store.put(&s, cookies(&[("SID", "abc"), ("XSRF", "def")])).unwrap();
    client
        .execute(client.request(Method::POST, server.url("/cookies")))
        .await
        .unwrap();

    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        header_value(&requests[0], "authorization").unwrap(),
        format!("Bearer {SENTINEL}")
    );
    assert_eq!(header_value(&requests[0], "cookie"), None, "caller cookie removed");
    assert_eq!(
        header_value(&requests[1], "authorization").unwrap(),
        "Basic amRvZTpwYXNzIHdvcmQ=",
        "base64 of jdoe:pass word"
    );
    assert_eq!(header_value(&requests[2], "cookie").unwrap(), "SID=abc; XSRF=def");
    assert_eq!(header_value(&requests[2], "authorization"), None);
}

// Exigence : EX-002-06
#[tokio::test]
async fn redirect_to_another_origin_is_not_followed() {
    let elsewhere = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = format!("http://{}/stolen", elsewhere.local_addr().unwrap());
    let server = TestServer::start(format!(
        "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    ))
    .await;
    let s = sid("sso-corp");
    let store = Arc::new(store());
    store.put(&s, bearer(SENTINEL)).unwrap();
    let client = client_for(&s, &store, &[&server.origin()]);

    let response = client
        .execute(client.request(Method::GET, server.url("/")))
        .await
        .unwrap();
    assert_eq!(response.status(), 302);
    assert_eq!(server.requests().len(), 1);
    assert_no_connection(&elsewhere).await;
}

// Exigence : EX-002-06
#[tokio::test]
async fn credential_headers_are_removed_from_the_response() {
    let server = TestServer::start(ok_response(
        "Set-Cookie: SID=new-session-value; HttpOnly\r\nSet-Cookie: other=1\r\nAuthorization: Bearer echoed\r\n\
         WWW-Authenticate: Bearer realm=\"x\"\r\nProxy-Authenticate: Basic\r\nX-Kept: yes\r\n",
    ))
    .await;
    let s = sid("sso-corp");
    let store = Arc::new(store());
    store.put(&s, bearer(SENTINEL)).unwrap();
    let client = client_for(&s, &store, &[&server.origin()]);

    let response = client
        .execute(client.request(Method::GET, server.url("/")))
        .await
        .unwrap();
    let headers = response.headers();
    for name in ["set-cookie", "authorization", "www-authenticate", "proxy-authenticate"] {
        assert!(!headers.contains_key(name), "{name} kept");
    }
    assert_eq!(headers.get("x-kept").unwrap(), "yes");
}

// Exigence : EX-002-06
#[tokio::test]
async fn undeclared_origin_is_refused_before_sending() {
    let declared = TestServer::start(ok_response("")).await;
    let undeclared = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/", undeclared.local_addr().unwrap())).unwrap();
    let s = sid("sso-corp");
    let store = Arc::new(store());
    store.put(&s, bearer(SENTINEL)).unwrap();
    let client = client_for(&s, &store, &[&declared.origin()]);

    let error = client.execute(client.request(Method::GET, url)).await.unwrap_err();
    assert!(matches!(error, SecretError::OriginNotAllowed { .. }), "{error:?}");
    assert_no_connection(&undeclared).await;
    assert!(declared.requests().is_empty());
}

#[tokio::test]
async fn session_without_secret_sends_nothing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let s = sid("sso-corp");
    let store = Arc::new(store());
    let client = client_for(&s, &store, &[&origin]);

    let url = Url::parse(&format!("{origin}/")).unwrap();
    let error = client.execute(client.request(Method::GET, url)).await.unwrap_err();
    assert_eq!(error, SecretError::NoSecret(s));
    assert_no_connection(&listener).await;
}

#[tokio::test]
async fn request_timeout_applies() {
    // Accepts connections but never answers.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let held = Arc::new(Mutex::new(Vec::new()));
    let keep = Arc::clone(&held);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            keep.lock().unwrap().push(stream);
        }
    });
    let s = sid("sso-corp");
    let store = Arc::new(store());
    store.put(&s, bearer(SENTINEL)).unwrap();
    let client = AuthenticatedHttpClient::new(
        s.clone(),
        Arc::clone(&store),
        Arc::new(guard_for(&s, &[&origin])),
        Duration::from_millis(300),
    )
    .unwrap();

    let url = Url::parse(&format!("{origin}/")).unwrap();
    let error = client.execute(client.request(Method::GET, url)).await.unwrap_err();
    assert_eq!(error, SecretError::Timeout);
}

#[test]
fn client_requires_a_known_session() {
    let store = Arc::new(store());
    let guard = Arc::new(guard_for(&sid("sso-corp"), &["https://mcp.example.internal"]));
    assert_eq!(
        AuthenticatedHttpClient::new(sid("sso-prod"), store, guard, Duration::from_secs(5)).unwrap_err(),
        SecretError::UnknownSession(sid("sso-prod"))
    );
}
