use std::collections::BTreeSet;

use proptest::prelude::*;
use secrecy::SecretString;
use url::Url;

use super::sealed::drop_probe;
use super::*;
use crate::config::{Config, Origin, SessionId};

const SENTINEL: &str = "S3NT1NEL-6f1c2b9e-do-not-print";

/// Test double that really transforms the buffer, so that reading a sealed buffer as clear text would be noticed.
struct XorProtector;

impl MemoryProtector for XorProtector {
    fn protect(&self, buf: &mut [u8]) -> Result<(), SecretError> {
        check_block_len(buf)?;
        buf.iter_mut().for_each(|b| *b ^= 0xa5);
        Ok(())
    }

    fn unprotect(&self, buf: &mut [u8]) -> Result<(), SecretError> {
        self.protect(buf)
    }
}

/// Protector whose decryption always fails.
struct BrokenUnprotect;

impl MemoryProtector for BrokenUnprotect {
    fn protect(&self, buf: &mut [u8]) -> Result<(), SecretError> {
        check_block_len(buf)
    }

    fn unprotect(&self, _: &mut [u8]) -> Result<(), SecretError> {
        Err(SecretError::Protection(ProtectionFailure::Os(-1)))
    }
}

fn sid(id: &str) -> SessionId {
    SessionId::new(id).unwrap()
}

fn store() -> SecretStore {
    SecretStore::new(Box::new(XorProtector))
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

// --- Memory protection ---------------------------------------------------------------------------------------------

#[cfg(not(windows))]
#[test]
fn insecure_dev_protector_round_trip_and_alignment() {
    let protector = InsecureDevProtector::new();
    let original: Vec<u8> = (0u8..32).collect();
    let mut buf = original.clone();
    protector.protect(&mut buf).unwrap();
    protector.unprotect(&mut buf).unwrap();
    assert_eq!(buf, original);
    let misaligned = Err(SecretError::Protection(ProtectionFailure::Misaligned));
    assert_eq!(protector.protect(&mut [0u8; 17]), misaligned);
    assert_eq!(protector.unprotect(&mut []), misaligned);
}

#[test]
fn sealed_secret_round_trip_with_padding() {
    for len in [0usize, 1, 15, 16, 17, 31, 32, 100] {
        let value: Vec<u8> = (0..len).map(|i| (i % 251) as u8 + 1).collect();
        let sealed = SealedSecret::seal(&XorProtector, &value).unwrap();
        assert_eq!(sealed.len(), len);
        assert_eq!(sealed.is_empty(), len == 0);
        assert_eq!(sealed.unseal(&XorProtector).unwrap().as_bytes(), value.as_slice());
        assert_eq!(
            sealed.unseal(&XorProtector).unwrap().as_bytes(),
            value.as_slice(),
            "unseal works on a copy"
        );
    }
}

#[test]
fn sealed_secret_refuses_oversized_values() {
    let value = vec![b'a'; MAX_SECRET_LEN + 1];
    assert_eq!(
        SealedSecret::seal(&XorProtector, &value).unwrap_err(),
        SecretError::Protection(ProtectionFailure::TooLarge)
    );
}

#[test]
fn platform_store_round_trip() {
    let store = SecretStore::for_platform();
    let s = sid("sso-corp");
    store.put(&s, bearer(SENTINEL)).unwrap();
    assert_eq!(token_of(&store, &s).unwrap(), SENTINEL);
}

#[test]
fn sealed_buffer_is_zeroed_on_drop() {
    let sealed = SealedSecret::seal(&XorProtector, SENTINEL.as_bytes()).unwrap();
    drop_probe::take();
    drop(sealed);
    let last = drop_probe::take().unwrap();
    assert_eq!(last.len(), 32);
    assert!(last.iter().all(|b| *b == 0));
}

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
    let sealed = SealedSecret::seal(&XorProtector, value.as_bytes()).unwrap();
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
    let broken = SecretStore::new(Box::new(BrokenUnprotect));
    broken.put(&s, bearer(value)).unwrap();
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

#[test]
fn no_secret_in_debug_or_display() {
    for output in outputs_with_secret(SENTINEL) {
        assert!(!output.contains(SENTINEL), "secret leaked in {output}");
        assert!(!output.contains("6f1c2b9e"), "part of the secret leaked in {output}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn no_secret_in_any_output(value in "[!-:<-~]{12,64}") {
        for output in outputs_with_secret(&value) {
            prop_assert!(!output.contains(&value), "secret leaked in {}", output);
        }
    }
}

// --- Store -----------------------------------------------------------------------------------------------------------

#[test]
fn put_replaces_the_session_secret() {
    let store = store();
    let s = sid("sso-corp");
    assert!(!store.has(&s));
    store.put(&s, bearer("first-token")).unwrap();
    store.put(&s, bearer("second-token")).unwrap();
    assert!(store.has(&s));
    assert_eq!(token_of(&store, &s).unwrap(), "second-token");
    store
        .with_variants(|v| assert_eq!(v, [b"second-token".as_slice()]))
        .unwrap();
}

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
    store.with_variants(|v| assert!(v.is_empty())).unwrap();
}

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
fn variants_are_the_raw_values_of_every_active_secret() {
    let store = store();
    store.put(&sid("sso-corp"), bearer("bearer-value")).unwrap();
    store.put(&sid("sso-tools"), basic("jdoe", "basic-password")).unwrap();
    store
        .put(
            &sid("sso-prod"),
            cookies(&[("A", "cookie-1"), ("B", "cookie-2"), ("C", "cookie-1")]),
        )
        .unwrap();
    let mut variants = store
        .with_variants(|v| {
            v.iter()
                .map(|n| String::from_utf8(n.to_vec()).unwrap())
                .collect::<Vec<_>>()
        })
        .unwrap();
    variants.sort();
    assert_eq!(variants, ["basic-password", "bearer-value", "cookie-1", "cookie-2"]);
}

#[test]
fn with_variants_fails_closed() {
    let store = SecretStore::new(Box::new(BrokenUnprotect));
    store.put(&sid("sso-corp"), bearer("token")).unwrap();
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
