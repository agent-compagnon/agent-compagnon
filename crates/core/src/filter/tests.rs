use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use proptest::prelude::*;
use proptest::sample::Index;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::*;
use crate::config::{Config, FilterConfig, SessionId};
use crate::secrets::test_support::TestWrapper;
use crate::secrets::variants::{JsonEscape, aligned_base64, derive_variants, hex, json_escape, percent_encode};
use crate::secrets::{Clock, Credential, SecretError, SecretStore};

const SENTINEL: &str = "S3NT1NEL-6f1c2b9e-do-not-print";

/// A real (signed with a throw-away key) HS256 JWT.
const JWT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJqZG9lIiwiZXhwIjoxNzAwMDAwMDAwfQ.\
                   Zb2xqzD8aU0pLwH4yqXk0mJk5T0Qm1cJ3a8W9eXwYfI";

struct ManualClock(Mutex<Instant>);

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

fn sid(id: &str) -> SessionId {
    SessionId::new(id).unwrap()
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

fn filter_for(credentials: Vec<Credential>) -> OutputFilter {
    let store = Arc::new(SecretStore::new(Box::new(TestWrapper::default())).unwrap());
    for (i, credential) in credentials.into_iter().enumerate() {
        store.put(&sid(&format!("session-{i}")), credential).unwrap();
    }
    OutputFilter::new(store, &FilterConfig::default()).unwrap()
}

fn filter_with_config(credentials: Vec<Credential>, config: &FilterConfig) -> OutputFilter {
    let store = Arc::new(SecretStore::new(Box::new(TestWrapper::default())).unwrap());
    for (i, credential) in credentials.into_iter().enumerate() {
        store.put(&sid(&format!("session-{i}")), credential).unwrap();
    }
    OutputFilter::new(store, config).unwrap()
}

fn text(filter: &OutputFilter, input: &str) -> String {
    filter.filter_text(input).unwrap().0.into_owned()
}

fn report(filter: &OutputFilter, input: &str) -> FilterReport {
    filter.filter_text(input).unwrap().1
}

fn json_out(filter: &OutputFilter, mut value: Value) -> (Value, FilterReport) {
    let report = filter.filter_json(&mut value).unwrap();
    (value, report)
}

fn variants_of(credential: &Credential) -> Vec<String> {
    derive_variants(credential)
        .iter()
        .map(|v| String::from_utf8(v.to_vec()).unwrap())
        .collect()
}

fn s(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Every key, string and number of a document, as text.
fn texts(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![value];
    while let Some(node) = stack.pop() {
        match node {
            Value::String(s) => out.push(s.clone()),
            Value::Number(n) => out.push(n.to_string()),
            Value::Array(items) => stack.extend(items),
            Value::Object(map) => {
                out.extend(map.keys().cloned());
                stack.extend(map.values());
            }
            _ => {}
        }
    }
    out
}

// --- Variant derivation ----------------------------------------------------------------------------------------------

#[test]
fn alignment_fragments_follow_the_spec_formula() {
    // 3 bytes at offset 0: the 4 characters; offset 1: characters 2..5; offset 2: characters 3..6.
    let value = b"abc";
    assert_eq!(s(&aligned_base64(&STANDARD_NO_PAD, value, 0)), "YWJj");
    let at1 = STANDARD_NO_PAD.encode(b"\0abc");
    assert_eq!(s(&aligned_base64(&STANDARD_NO_PAD, value, 1)), at1[2..5]);
    let at2 = STANDARD_NO_PAD.encode(b"\0\0abc");
    assert_eq!(s(&aligned_base64(&STANDARD_NO_PAD, value, 2)), at2[3..6]);
}

#[test]
fn variants_are_deduplicated_and_short_ones_are_raw() {
    for (credential, raws) in [
        (bearer("plain-token-1234"), vec!["plain-token-1234"]),
        (bearer(JWT), vec![JWT]),
        (basic("jdoe", "pw"), vec!["pw"]),
        (cookies(&[("SID", "a%2Fb"), ("XSRF", "a%2Fb")]), vec!["a%2Fb", "a/b"]),
    ] {
        let variants = variants_of(&credential);
        for (i, v) in variants.iter().enumerate() {
            assert!(!variants[..i].contains(v), "{v}");
            assert!(
                v.len() >= crate::secrets::variants::MIN_DERIVED_LEN || raws.contains(&v.as_str()),
                "{v}"
            );
        }
        for raw in raws {
            assert!(variants.contains(&raw.to_owned()), "{raw}");
        }
    }
}

// --- EX-003-01: base64 at every alignment ------------------------------------------------------------------------

proptest! {
    // Exigence : EX-003-01
    #[test]
    fn buried_secret_is_found_in_base64_at_every_offset(
        secret_value in "[!-~]{8,48}",
        blocks in 0usize..4,
        offset in 0usize..3,
        filler in proptest::collection::vec(any::<u8>(), 12),
        suffix in proptest::collection::vec(any::<u8>(), 0..12),
        url_safe in any::<bool>(),
        padded in any::<bool>(),
    ) {
        let filter = filter_for(vec![bearer(&secret_value)]);
        let mut buf = filler[..3 * blocks + offset].to_vec();
        buf.extend_from_slice(secret_value.as_bytes());
        buf.extend_from_slice(&suffix);
        let encoded = match (url_safe, padded) {
            (false, true) => STANDARD.encode(&buf),
            (false, false) => STANDARD_NO_PAD.encode(&buf),
            (true, true) => URL_SAFE.encode(&buf),
            (true, false) => URL_SAFE_NO_PAD.encode(&buf),
        };
        let fragment = if url_safe {
            aligned_base64(&URL_SAFE_NO_PAD, secret_value.as_bytes(), offset)
        } else {
            aligned_base64(&STANDARD_NO_PAD, secret_value.as_bytes(), offset)
        };
        let fragment = s(&fragment);
        prop_assert!(encoded.contains(&fragment));

        let input = format!("image: data:image/png;base64,{encoded} fin");
        let (out, report) = filter.filter_text(&input).unwrap();
        prop_assert!(!out.contains(&fragment));
        prop_assert!(out.contains(SECRET_MARKER));
        prop_assert!(report.exact_hits >= 1);

        let (doc, _) = json_out(&filter, json!({ "content": [{ "type": "image", "data": encoded }] }));
        prop_assert!(!texts(&doc).iter().any(|t| t.contains(&fragment)));
    }
}

// Exigence : EX-003-01
#[test]
fn base64_without_the_secret_is_untouched() {
    let filter = filter_for(vec![bearer("the-secret-token-value")]);
    let other = STANDARD.encode(b"\x01\x02some other bytes entirely, no secret here");
    assert_eq!(text(&filter, &other), other);
    let other = URL_SAFE_NO_PAD.encode(b"\xffthe-secret-token-valu");
    assert_eq!(text(&filter, &other), other);
}

// --- EX-003-02: hexadecimal, percent-encoding, JSON escaping ---------------------------------------------------------

const SPECIAL: &str = "p@ss/w:rd+é \"q\" \\x";

// Exigence : EX-003-02
#[test]
fn hexadecimal_in_both_cases_is_masked() {
    let filter = filter_for(vec![bearer(SPECIAL)]);
    for upper in [false, true] {
        let encoded = s(&hex(SPECIAL.as_bytes(), upper));
        let out = text(&filter, &format!("dump {encoded} end"));
        assert_eq!(out, format!("dump {SECRET_MARKER} end"));
    }
    let unrelated = s(&hex(b"p@ss/w:rd+e \"q\" \\x", false));
    assert_eq!(text(&filter, &unrelated), unrelated);
}

// Exigence : EX-003-02
#[test]
fn percent_encoding_in_both_cases_and_form_encoding_are_masked() {
    let filter = filter_for(vec![bearer(SPECIAL)]);
    for upper in [false, true] {
        for form in [false, true] {
            let encoded = s(&percent_encode(SPECIAL.as_bytes(), upper, form));
            assert!(encoded.contains('%'));
            let out = text(&filter, &format!("https://h.example/?q={encoded}&x=1"));
            assert_eq!(out, format!("https://h.example/?q={SECRET_MARKER}&x=1"), "{encoded}");
        }
    }
    let unrelated = "https://h.example/?q=p%40ss%2Fw%3Ard%2Be&x=1";
    assert_eq!(text(&filter, unrelated), unrelated);
}

// Exigence : EX-003-02
#[test]
fn json_escaped_forms_are_masked() {
    let filter = filter_for(vec![bearer(SPECIAL)]);
    for form in [JsonEscape::Minimal, JsonEscape::Solidus, JsonEscape::Ascii] {
        let escaped = json_escape(SPECIAL, form).to_string();
        assert_ne!(escaped, SPECIAL);
        let input = format!(r#"{{"note": "{escaped}"}}"#);
        assert_eq!(text(&filter, &input), format!(r#"{{"note": "{SECRET_MARKER}"}}"#));
    }
    // The text of a JSON document produced by serde_json.
    let serialized = serde_json::to_string(&json!({ "note": SPECIAL })).unwrap();
    assert!(!text(&filter, &serialized).contains(&json_escape(SPECIAL, JsonEscape::Minimal).to_string()));
    let unrelated = r#"{"note": "p@ss/w:rd+e \"q\""}"#;
    assert_eq!(text(&filter, unrelated), unrelated);
}

// --- EX-003-03: JWT signature, Basic, cookies ------------------------------------------------------------------------

// Exigence : EX-003-03
#[test]
fn jwt_signature_alone_is_masked_in_every_encoding() {
    let filter = filter_for(vec![bearer(JWT)]);
    let signature = JWT.rsplit('.').next().unwrap();
    assert_eq!(
        text(&filter, &format!("sig={signature};")),
        format!("sig={SECRET_MARKER};")
    );
    let bytes = URL_SAFE_NO_PAD.decode(signature).unwrap();
    for encoded in [STANDARD.encode(&bytes), s(&hex(&bytes, false)), s(&hex(&bytes, true))] {
        let out = text(&filter, &format!("sig {encoded} ok"));
        assert_eq!(out, format!("sig {SECRET_MARKER} ok"), "{encoded}");
    }
    // Another signature of the same length is not a known secret.
    let other = "Ab2xqzD8aU0pLwH4yqXk0mJk5T0Qm1cJ3a8W9eXwYfZ";
    assert_eq!(text(&filter, other), other);
}

// Exigence : EX-003-03
#[test]
fn basic_login_password_and_password_variants_are_masked() {
    let filter = filter_for(vec![basic("jdoe", "Corr3ct-Horse")]);
    let user_pass = STANDARD.encode("jdoe:Corr3ct-Horse");
    assert_eq!(
        text(&filter, &format!("x {user_pass} y")),
        format!("x {SECRET_MARKER} y")
    );
    assert_eq!(
        report(&filter, &format!("x {user_pass} y")).kinds,
        [HitKind::KnownSecret]
    );
    for variant in [
        "Corr3ct-Horse".to_owned(),
        s(&hex(b"Corr3ct-Horse", true)),
        STANDARD.encode("Corr3ct-Horse"),
    ] {
        assert_eq!(text(&filter, &format!("[{variant}]")), format!("[{SECRET_MARKER}]"));
    }
    // The login alone is not a secret.
    assert_eq!(text(&filter, "user jdoe"), "user jdoe");
    let other = STANDARD.encode("jdoe:another-pass");
    assert_eq!(text(&filter, &other), other);
}

// Exigence : EX-003-03
#[test]
fn cookie_values_and_their_decoded_form_are_masked() {
    let filter = filter_for(vec![cookies(&[
        ("SID", "s%3Aabc%2Fdef.ghi"),
        ("XSRF", "xsrf-value-123"),
    ])]);
    assert_eq!(text(&filter, "v=s%3Aabc%2Fdef.ghi"), format!("v={SECRET_MARKER}"));
    assert_eq!(text(&filter, "v=s:abc/def.ghi"), format!("v={SECRET_MARKER}"));
    assert_eq!(
        text(&filter, "token xsrf-value-123 !"),
        format!("token {SECRET_MARKER} !")
    );
    assert_eq!(text(&filter, "v=s:abc/def.gh"), "v=s:abc/def.gh");
}

// --- EX-003-04: positions in a document ------------------------------------------------------------------------------

// Exigence : EX-003-04
#[test]
fn secret_is_masked_wherever_it_is_in_a_document() {
    let filter = filter_for(vec![bearer(SENTINEL)]);
    let (doc, report) = json_out(
        &filter,
        json!({
            format!("key-{SENTINEL}"): 1,
            "list": [1, ["a", ["b", SENTINEL]]],
            "structuredContent": { "a": { "b": { "c": { "d": [{ "e": format!("deep {SENTINEL} value") }] } } } },
            "text": format!("Voici le jeton {SENTINEL}, à ne pas répéter."),
        }),
    );
    assert_eq!(
        doc,
        json!({
            format!("key-{SECRET_MARKER}"): 1,
            "list": [1, ["a", ["b", SECRET_MARKER]]],
            "structuredContent": { "a": { "b": { "c": { "d": [{ "e": format!("deep {SECRET_MARKER} value") }] } } } },
            "text": format!("Voici le jeton {SECRET_MARKER}, à ne pas répéter."),
        })
    );
    assert_eq!(report.exact_hits, 4);
    assert_eq!(report.kinds, [HitKind::KnownSecret]);
}

// Exigence : EX-003-04
#[test]
fn colliding_keys_are_grouped_instead_of_renamed() {
    let filter = filter_for(vec![bearer(SENTINEL), bearer("second-secret-value")]);
    let (doc, _) = json_out(&filter, json!({ SENTINEL: 1, "second-secret-value": 2 }));
    assert_eq!(doc, json!({ SECRET_MARKER: [1, 2] }));
}

// Exigence : EX-003-04
#[test]
fn numbers_holding_a_secret_are_masked() {
    let filter = filter_for(vec![basic("jdoe", "73914286")]);
    let (doc, _) = json_out(&filter, json!({ "pin": 73914286, "n": 12, "big": 1739142860 }));
    assert_eq!(
        doc,
        json!({ "pin": SECRET_MARKER, "n": 12, "big": format!("1{SECRET_MARKER}0") })
    );
}

#[test]
fn deeply_nested_documents_do_not_overflow() {
    let filter = filter_for(vec![bearer(SENTINEL)]);
    let mut doc = Value::String(SENTINEL.to_owned());
    for _ in 0..100_000 {
        doc = Value::Array(vec![doc]);
    }
    filter.filter_json(&mut doc).unwrap();
    let mut node = &doc;
    while let Value::Array(items) = node {
        node = &items[0];
    }
    assert_eq!(node, &Value::String(SECRET_MARKER.to_owned()));
    // Dropping such a document recurses in serde_json itself: unwrap it iteratively.
    let mut node = doc;
    while let Value::Array(mut items) = node {
        node = items.pop().unwrap();
    }
}

// --- EX-003-05: short secrets ----------------------------------------------------------------------------------------

// Exigence : EX-003-05
#[test]
fn short_secret_raw_value_masked_derived_variants_ignored() {
    let credential = bearer("k3y");
    let variants = variants_of(&credential);
    assert!(variants.contains(&"k3y".to_owned()));
    assert!(
        variants
            .iter()
            .all(|v| v == "k3y" || v.len() >= crate::secrets::variants::MIN_DERIVED_LEN)
    );
    // Hex (6 characters) and base64 (4 characters) are too short to be kept.
    assert!(!variants.contains(&"6b3379".to_owned()));
    assert!(!variants.contains(&"azN5".to_owned()));

    let filter = filter_for(vec![credential]);
    assert_eq!(text(&filter, "a k3y b"), format!("a {SECRET_MARKER} b"));
    assert_eq!(text(&filter, "6b3379 azN5"), "6b3379 azN5");
}

// Exigence : EX-003-05
#[test]
fn derived_variant_of_eight_characters_is_kept() {
    // 6 bytes give 8 base64 characters and 12 hex digits.
    let variants = variants_of(&bearer("abcdef"));
    assert!(variants.contains(&"YWJjZGVm".to_owned()));
    assert!(variants.contains(&"616263646566".to_owned()));
}

// --- EX-003-06: generic patterns -------------------------------------------------------------------------------------

// Exigence : EX-003-06
#[test]
fn unknown_jwt_is_masked() {
    let filter = filter_for(vec![]);
    let other = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJlLW5vdC1rbm93bg";
    assert_eq!(text(&filter, &format!("jwt={other}.")), format!("jwt={JWT_MARKER}."));
    let jwe = "eyJhbGciOiJSU0EtT0FFUCJ9.ZW5jcnlwdGVkLWtleQ.aXY.Y2lwaGVydGV4dA.dGFn";
    assert_eq!(text(&filter, jwe), JWT_MARKER);
    assert_eq!(report(&filter, jwe).kinds, [HitKind::Jwt]);
    for intact in [
        "eyJ",
        "eyJhbGciOiJIUzI1NiJ9",
        "eyJhbGciOiJIUzI1NiJ9.only-two",
        "abc.def.ghi",
    ] {
        assert_eq!(text(&filter, intact), intact);
    }
}

// Exigence : EX-003-06
#[test]
fn authorization_headers_are_masked() {
    let filter = filter_for(vec![]);
    for (input, expected) in [
        (
            "Authorization: Bearer abc.def-123",
            format!("Authorization: Bearer {HEADER_MARKER}"),
        ),
        (
            "authorization: basic amRvZTpwdw==\r\nHost: x",
            format!("authorization: basic {HEADER_MARKER}\r\nHost: x"),
        ),
        (
            "Authorization: Negotiate YIIabc",
            format!("Authorization: Negotiate {HEADER_MARKER}"),
        ),
        (
            "Authorization: NTLM TlRMTVNTUAAB",
            format!("Authorization: NTLM {HEADER_MARKER}"),
        ),
        (
            r#"Authorization: Digest username="jdoe", response="6629fae4""#,
            format!("Authorization: Digest {HEADER_MARKER}"),
        ),
        (
            "Proxy-Authorization: Bearer zzz",
            format!("Proxy-Authorization: Bearer {HEADER_MARKER}"),
        ),
        (
            "GET /?authorization=abcdef&x=1",
            format!("GET /?authorization={HEADER_MARKER}&x=1"),
        ),
        (
            "authorization=Bearer abcdef rest",
            format!("authorization=Bearer {HEADER_MARKER} rest"),
        ),
    ] {
        assert_eq!(text(&filter, input), expected, "{input}");
    }
    assert_eq!(
        report(&filter, "Authorization: Bearer abc").kinds,
        [HitKind::AuthorizationHeader]
    );
    // An Authorization header whose value is a JWT is masked whole by the JWT pattern first.
    assert_eq!(
        text(&filter, &format!("Authorization: Bearer {JWT}")),
        format!("Authorization: Bearer {JWT_MARKER}")
    );
}

// Exigence : EX-003-06
#[test]
fn cookie_header_lines_are_masked() {
    let filter = filter_for(vec![]);
    for (input, expected) in [
        (
            "Set-Cookie: SID=abc; Path=/; HttpOnly",
            format!("Set-Cookie: {HEADER_MARKER}"),
        ),
        ("set-cookie2: a=b", format!("set-cookie2: {HEADER_MARKER}")),
        (
            "Cookie: SID=abc; XSRF=def\nnext",
            format!("Cookie: {HEADER_MARKER}\nnext"),
        ),
    ] {
        assert_eq!(text(&filter, input), expected, "{input}");
    }
    assert_eq!(report(&filter, "Cookie: a=b").kinds, [HitKind::CookieHeader]);
}

// Exigence : EX-003-06
#[test]
fn sensitive_json_fields_are_masked_only_when_strings() {
    let filter = filter_for(vec![]);
    let (doc, report) = json_out(
        &filter,
        json!({
            "password": "hunter2",
            "user": { "apiKey": "k-123", "X-Api-Key": "k-456", "client_secret": "cs", "accessKey": "ak" },
            "session_id": "abc",
            "id_token": "it",
            "refreshToken": "rt",
            "db_passwd": "p", "pwd": "p", "Credentials": "c", "my-secret": "s", "Authorization": "Bearer x",
            "set_cookie": "a=b",
            "password2": "p2",
            "token": 42,
            "secret": true,
            "credentials": { "nested": "visible" },
            "max_tokens": 4096,
            "token_count": 12,
            "tokenizer": "cl100k",
            "tokens": "a b c",
            "token_type": "Bearer",
            "secretary": "Alice",
        }),
    );
    assert_eq!(
        doc,
        json!({
            "password": FIELD_MARKER,
            "user": { "apiKey": FIELD_MARKER, "X-Api-Key": FIELD_MARKER, "client_secret": FIELD_MARKER, "accessKey": FIELD_MARKER },
            "session_id": FIELD_MARKER,
            "id_token": FIELD_MARKER,
            "refreshToken": FIELD_MARKER,
            "db_passwd": FIELD_MARKER, "pwd": FIELD_MARKER, "Credentials": FIELD_MARKER, "my-secret": FIELD_MARKER,
            "Authorization": FIELD_MARKER,
            "set_cookie": FIELD_MARKER,
            "password2": FIELD_MARKER,
            "token": 42,
            "secret": true,
            "credentials": { "nested": "visible" },
            "max_tokens": 4096,
            "token_count": 12,
            "tokenizer": "cl100k",
            "tokens": "a b c",
            "token_type": "Bearer",
            "secretary": "Alice",
        })
    );
    assert_eq!(report.generic_hits, 15);
    assert_eq!(report.kinds, [HitKind::SensitiveField]);
}

// Exigence : EX-003-06
#[test]
fn sensitive_text_fields_are_masked() {
    let filter = filter_for(vec![]);
    for (input, expected) in [
        ("password=hunter2&user=bob", format!("password={FIELD_MARKER}&user=bob")),
        (
            "db.password = 'two words' ok",
            format!("db.password = '{FIELD_MARKER}' ok"),
        ),
        (
            "api_key: k-123 456\nnext: 1",
            format!("api_key: {FIELD_MARKER}\nnext: 1"),
        ),
        ("X-Api-Key: abc", format!("X-Api-Key: {FIELD_MARKER}")),
        (
            r#"{"password": "a\"b", "user": "bob"}"#,
            format!(r#"{{"password": "{FIELD_MARKER}", "user": "bob"}}"#),
        ),
        ("{'token': 'abc'}", format!("{{'token': '{FIELD_MARKER}'}}")),
        (
            r#"{\"client_secret\":\"xyz\"}"#,
            format!(r#"{{\"client_secret\":\"{FIELD_MARKER}\"}}"#),
        ),
        ("?access_token=abc.def;x", format!("?access_token={FIELD_MARKER};x")),
    ] {
        assert_eq!(text(&filter, input), expected, "{input}");
    }
    for intact in [
        "max_tokens: 4096",
        "token_count=12",
        "tokenizer: cl100k",
        r#"{"max_tokens": 4096, "token_count": 12, "tokenizer": "cl100k", "token": 7}"#,
        "password:",
        "secretary: Alice",
    ] {
        assert_eq!(text(&filter, intact), intact);
    }
}

// Exigence : EX-003-06
#[test]
fn private_keys_and_url_passwords_are_masked() {
    let filter = filter_for(vec![]);
    let pem = "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIEow\nIBAAK\n-----END RSA PRIVATE KEY-----\nafter";
    assert_eq!(text(&filter, pem), format!("before\n{SECRET_MARKER}\nafter"));
    let unterminated = "x -----BEGIN PRIVATE KEY-----\nMIIEow";
    assert_eq!(text(&filter, unterminated), format!("x {SECRET_MARKER}"));
    let public = "-----BEGIN PUBLIC KEY-----\nMIIBIj\n-----END PUBLIC KEY-----";
    assert_eq!(text(&filter, public), public);

    assert_eq!(
        text(&filter, "postgres://app:p@ss@db.example:5432/x"),
        format!("postgres://app:{SECRET_MARKER}@db.example:5432/x")
    );
    assert_eq!(report(&filter, "https://u:pw@h/").kinds, [HitKind::UrlCredentials]);
    assert_eq!(
        text(&filter, "https://h.example:8443/a@b"),
        "https://h.example:8443/a@b"
    );
}

// Exigence : EX-003-06
#[test]
fn markers_never_trigger_a_pattern() {
    let filter = filter_for(vec![bearer("SECRET-1234"), bearer("MASQU")]);
    let input = MARKERS.join(" ");
    let (out, report) = filter.filter_text(&input).unwrap();
    assert_eq!(out, input);
    assert!(report.is_clean());
    let fields = format!(
        "password={FIELD_MARKER}\npassword: {FIELD_MARKER}\nAuthorization: Bearer {HEADER_MARKER}\nCookie: {HEADER_MARKER}\n\
         https://u:{SECRET_MARKER}@h/ {JWT_MARKER}"
    );
    assert_eq!(text(&filter, &fields), fields);
}

// --- EX-003-07: idempotence ------------------------------------------------------------------------------------------

// Exigence : EX-003-07
#[test]
fn filtering_twice_gives_the_same_result() {
    let filter = filter_for(vec![bearer(SENTINEL), basic("jdoe", "Corr3ct-Horse")]);
    let input = format!(
        "Authorization: Bearer {SENTINEL}\npassword=[CHAMP foo {JWT}\nhttps://u:{SENTINEL}@h\n\
         x{SENTINEL}{SENTINEL}y {} \"token\": \"[SECRET MASQUÉ]x\"",
        STANDARD.encode("jdoe:Corr3ct-Horse")
    );
    let once = text(&filter, &input);
    assert_eq!(text(&filter, &once), once);
    assert!(!once.contains(SENTINEL));

    let doc = json!({ SENTINEL: { "password": SENTINEL, "x": [format!("a{SENTINEL}"), 73914286] } });
    let (once, _) = json_out(&filter, doc);
    let (twice, report) = json_out(&filter, once.clone());
    assert_eq!(once, twice);
    assert!(report.is_clean());
}

fn piece() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(SENTINEL.to_owned()),
        Just(STANDARD.encode(SENTINEL)),
        Just("Authorization: Bearer ".to_owned()),
        Just("Cookie: ".to_owned()),
        Just("password=".to_owned()),
        Just("\"token\": \"".to_owned()),
        Just("api_key: ".to_owned()),
        Just("https://user:".to_owned()),
        Just("@host/".to_owned()),
        Just(JWT.to_owned()),
        Just("-----BEGIN PRIVATE KEY-----".to_owned()),
        Just("-----END PRIVATE KEY-----".to_owned()),
        proptest::sample::select(MARKERS.to_vec()).prop_map(str::to_owned),
        "[ -~]{0,12}",
        Just("\n".to_owned()),
        Just("[".to_owned()),
        Just("é".to_owned()),
    ]
}

proptest! {
    // Exigence : EX-003-07
    #[test]
    fn filtering_is_idempotent(pieces in proptest::collection::vec(piece(), 0..24)) {
        let filter = filter_for(vec![bearer(SENTINEL)]);
        let input = pieces.concat();
        let once = text(&filter, &input);
        prop_assert_eq!(text(&filter, &once), once.clone());
        prop_assert!(!once.contains(SENTINEL));
        let (doc, _) = json_out(&filter, json!({ input.clone(): [input] }));
        let (again, report) = json_out(&filter, doc.clone());
        prop_assert_eq!(doc, again);
        prop_assert!(report.is_clean());
    }
}

// --- EX-003-08: incoming arguments -----------------------------------------------------------------------------------

// Exigence : EX-003-08
#[test]
fn contains_known_secret_is_true_for_every_variant() {
    let credentials = || {
        vec![
            bearer(JWT),
            basic("jdoe", SPECIAL),
            cookies(&[("SID", "s%3Aabc%2Fdef.ghi")]),
        ]
    };
    let filter = filter_for(credentials());
    let mut variants = Vec::new();
    for credential in credentials() {
        variants.extend(variants_of(&credential));
    }
    assert!(variants.len() > 40);
    for variant in &variants {
        for args in [
            json!({ "query": format!("x {variant} y") }),
            json!({ "nested": [{ variant.clone(): 1 }] }),
        ] {
            assert!(filter.contains_known_secret(&args).unwrap(), "{variant}");
        }
    }
    let pin = filter_for(vec![basic("jdoe", "73914286")]);
    assert!(pin.contains_known_secret(&json!({ "n": 73914286 })).unwrap());
}

// Exigence : EX-003-08
#[test]
fn contains_known_secret_is_false_for_unknown_values() {
    let filter = filter_for(vec![bearer(JWT)]);
    let unknown = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJlLW5vdC1rbm93bg";
    assert!(
        !filter
            .contains_known_secret(&json!({ "jwt": unknown, "password": "x" }))
            .unwrap()
    );
    assert!(
        !filter
            .contains_known_secret(&json!({ "text": MARKERS.join(" ") }))
            .unwrap()
    );
    assert!(
        !filter_for(vec![])
            .contains_known_secret(&json!({ "jwt": JWT }))
            .unwrap()
    );
}

// --- EX-003-09: the report never holds the value ---------------------------------------------------------------------

fn outputs(filter: &OutputFilter, value: &str) -> Vec<String> {
    let input = format!("a {value} password={value} Authorization: Bearer {value}");
    let (_, report) = filter.filter_text(&input).unwrap();
    let (_, json_report) = json_out(filter, json!({ value: value, "token": value }));
    vec![
        format!("{report:?}"),
        format!("{json_report:?}"),
        format!("{filter:?}"),
        format!("{:?}", FilterError::Unavailable(SecretError::Corrupted)),
        FilterError::Unavailable(SecretError::Corrupted).to_string(),
        FilterError::Pattern.to_string(),
    ]
}

// Exigence : EX-003-09
#[test]
fn report_never_contains_the_value() {
    let filter = filter_for(vec![bearer(SENTINEL)]);
    for output in outputs(&filter, SENTINEL) {
        assert!(!output.contains(SENTINEL), "{output}");
    }
    let (_, report) = filter.filter_text(&format!("x {SENTINEL}")).unwrap();
    assert_eq!(
        report,
        FilterReport {
            exact_hits: 1,
            generic_hits: 0,
            kinds: vec![HitKind::KnownSecret]
        }
    );
}

proptest! {
    // Exigence : EX-003-09
    #[test]
    fn report_never_contains_any_value(value in "[!-~]{12,64}") {
        let filter = filter_for(vec![bearer(&value)]);
        for output in outputs(&filter, &value) {
            prop_assert!(!output.contains(&value));
        }
    }
}

// --- EX-003-10: logs -------------------------------------------------------------------------------------------------

#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl io::Write for SharedBuf {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl SharedBuf {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn log_with(filter: OutputFilter, emit: impl FnOnce()) -> String {
    let sink = SharedBuf::default();
    let writer_sink = sink.clone();
    let make_writer = FilteringMakeWriter::new(move || writer_sink.clone(), Arc::new(filter));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(make_writer)
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, emit);
    sink.contents()
}

// Exigence : EX-003-10
#[test]
fn make_writer_masks_a_secret_logged_with_info() {
    let logged = log_with(filter_for(vec![bearer(SENTINEL)]), || {
        tracing::info!(header = %format!("Bearer {SENTINEL}"), "appel amont {}", SENTINEL);
        tracing::info!("Set-Cookie: SID=abc; HttpOnly");
        tracing::warn!(encoded = %STANDARD.encode(SENTINEL), "deuxième ligne");
    });
    assert!(!logged.contains(SENTINEL), "{logged}");
    assert!(!logged.contains(&STANDARD.encode(SENTINEL)), "{logged}");
    assert!(!logged.contains("SID=abc"), "{logged}");
    assert_eq!(logged.lines().count(), 3, "{logged}");
    assert!(logged.contains(&format!("appel amont {SECRET_MARKER}")), "{logged}");
    assert!(logged.contains(&format!("Set-Cookie: {HEADER_MARKER}")), "{logged}");
}

// Exigence : EX-003-10
#[test]
fn writer_buffers_until_the_record_is_complete() {
    use std::io::Write as _;
    let sink = SharedBuf::default();
    let writer_sink = sink.clone();
    let make_writer = FilteringMakeWriter::new(
        move || writer_sink.clone(),
        Arc::new(filter_for(vec![bearer(SENTINEL)])),
    );
    {
        let mut writer = tracing_subscriber::fmt::MakeWriter::make_writer(&make_writer);
        let (head, tail) = SENTINEL.split_at(10);
        writer.write_all(format!("x {head}").as_bytes()).unwrap();
        writer
            .write_all(format!("{tail} y {}\n", "z".repeat(5000)).as_bytes())
            .unwrap();
        assert_eq!(sink.contents(), "");
    }
    let out = sink.contents();
    assert!(out.starts_with(&format!("x {SECRET_MARKER} y zzz")));
    assert!(!out.contains(SENTINEL));
}

// --- EX-003-11: general property -------------------------------------------------------------------------------------

fn document() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        "[ -~]{0,16}".prop_map(Value::String),
        "\\PC{0,8}".prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 48, 6, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            proptest::collection::btree_map("[a-zA-Z_]{1,10}", inner, 0..6)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

fn string_slots(value: &mut Value) -> Vec<&mut String> {
    let mut out = Vec::new();
    let mut stack = vec![value];
    while let Some(node) = stack.pop() {
        match node {
            Value::String(s) => out.push(s),
            Value::Array(items) => stack.extend(items.iter_mut()),
            Value::Object(map) => stack.extend(map.values_mut()),
            _ => {}
        }
    }
    out
}

fn credential_strategy() -> impl Strategy<Value = (u8, String, String)> {
    (0u8..3, "[!-~]{1,40}", "[a-z]{1,8}")
}

fn build((kind, value, login): &(u8, String, String)) -> Credential {
    match kind {
        0 => bearer(value),
        1 => basic(login, value),
        _ => cookies(&[("SID", &value.replace(';', ","))]),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // Exigence : EX-003-11
    #[test]
    fn no_variant_survives_in_any_document(
        spec in credential_strategy(),
        mut doc in document(),
        variant in any::<Index>(),
        slot in any::<Index>(),
        in_key in any::<bool>(),
        prefix in "[ -~]{0,6}",
        suffix in "[ -~]{0,6}",
    ) {
        // A secret that is part of a marker cannot be told apart from the marker.
        prop_assume!(!MARKERS.iter().any(|m| m.contains(spec.1.as_str())));
        let variants: Vec<Vec<u8>> = derive_variants(&build(&spec)).iter().map(|v| v.to_vec()).collect();
        // A percent-decoded cookie value may not be UTF-8: it can only be embedded when it is.
        let chosen = std::str::from_utf8(variant.get(&variants));
        prop_assume!(chosen.is_ok());
        let carrier = format!("{prefix}{}{suffix}", chosen.unwrap());
        if in_key {
            doc = json!({ carrier: doc });
        } else {
            let mut slots = string_slots(&mut doc);
            if slots.is_empty() {
                doc = json!([doc, carrier]);
            } else {
                let i = slot.index(slots.len());
                *slots.swap_remove(i) = carrier;
            }
        }
        let found = |t: &str| variants.iter().find(|v| memchr::memmem::find(t.as_bytes(), v).is_some()).cloned();

        let filter = filter_for(vec![build(&spec)]);
        let (out, report) = json_out(&filter, doc);
        prop_assert!(report.exact_hits >= 1);
        for t in texts(&out) {
            prop_assert_eq!(found(&t), None, "in {:?}", t);
        }
        // Text form of the same document.
        let serialized = serde_json::to_string(&out).unwrap();
        let (out, _) = filter.filter_text(&serialized).unwrap();
        prop_assert_eq!(found(&out), None);
    }
}

// --- EX-003-12: fail closed ------------------------------------------------------------------------------------------

fn broken_filter() -> OutputFilter {
    let wrapper = TestWrapper::default();
    let store = Arc::new(SecretStore::new(Box::new(wrapper.clone())).unwrap());
    store.put(&sid("sso-corp"), bearer(SENTINEL)).unwrap();
    wrapper.break_now();
    OutputFilter::new(store, &FilterConfig::default()).unwrap()
}

// Exigence : EX-003-12
#[test]
fn unreadable_blacklist_fails_closed() {
    let filter = broken_filter();
    let error = filter.filter_text(&format!("x {SENTINEL}")).unwrap_err();
    assert!(matches!(error, FilterError::Unavailable(_)));
    assert!(!error.to_string().contains(SENTINEL));

    let mut doc = json!({ "text": SENTINEL });
    assert!(filter.filter_json(&mut doc).is_err());
    assert_eq!(doc, Value::String(SECRET_MARKER.to_owned()));

    assert!(filter.contains_known_secret(&json!(SENTINEL)).is_err());
}

// Exigence : EX-003-12
#[test]
fn unreadable_blacklist_drops_log_records() {
    let logged = log_with(broken_filter(), || tracing::info!("jeton {}", SENTINEL));
    assert_eq!(logged, UNFILTERED_RECORD);
}

// --- EX-003-13: wiped secrets ----------------------------------------------------------------------------------------

// Exigence : EX-003-13
#[test]
fn wiped_secret_stays_masked_during_the_grace_period_only() {
    let clock = Arc::new(ManualClock(Mutex::new(Instant::now())));
    let grace = Duration::from_secs(130);
    let store = Arc::new(
        SecretStore::new(Box::new(TestWrapper::default()))
            .unwrap()
            .with_variant_grace(grace)
            .with_clock(clock.clone()),
    );
    let s = sid("sso-corp");
    store.put(&s, bearer(SENTINEL)).unwrap();
    let filter = OutputFilter::new(store.clone(), &FilterConfig::default()).unwrap();
    let late_response = format!("réponse tardive {SENTINEL} / {}", STANDARD.encode(SENTINEL));
    let masked = format!("réponse tardive {SECRET_MARKER} / {SECRET_MARKER}");

    store.wipe(&s);
    assert_eq!(text(&filter, &late_response), masked);
    *clock.0.lock().unwrap() += grace - Duration::from_secs(1);
    assert_eq!(text(&filter, &late_response), masked);
    *clock.0.lock().unwrap() += Duration::from_secs(1);
    assert_eq!(text(&filter, &late_response), late_response);

    store.put(&s, bearer("renewed-token-value")).unwrap();
    store.put(&s, bearer("renewed-again-value")).unwrap();
    assert_eq!(text(&filter, "renewed-token-value"), SECRET_MARKER);
}

// --- EX-003-14: [filter] in the configuration ------------------------------------------------------------------------

// Exigence : EX-003-14
#[test]
fn filter_section_adds_sensitive_keys() {
    let config = FilterConfig {
        extra_key_segments: vec!["passphrase".to_owned()],
        extra_keys: vec!["x-auth-key".to_owned()],
    };
    let filter = filter_with_config(vec![], &config);
    let (doc, _) = json_out(
        &filter,
        json!({ "gpg_passphrase": "p", "X_Auth_Key": "k", "password": "still masked", "x-auth": "visible" }),
    );
    assert_eq!(
        doc,
        json!({ "gpg_passphrase": FIELD_MARKER, "X_Auth_Key": FIELD_MARKER, "password": FIELD_MARKER, "x-auth": "visible" })
    );
    assert_eq!(text(&filter, "passphrase: abc"), format!("passphrase: {FIELD_MARKER}"));
}

fn embedded_with_filter(section: &str) -> String {
    crate::config::EMBEDDED_TOML.replace(
        "[filter]\nextra_key_segments = []             # dernier mot d'un nom de clé, ex. \"passphrase\"\n\
         extra_keys = []                     # nom de clé complet, ex. \"x-auth-key\"\n",
        section,
    )
}

// Exigence : EX-003-14
#[test]
fn filter_section_is_validated() {
    let embedded = Config::embedded().unwrap();
    assert_eq!(embedded.filter, FilterConfig::default());

    let toml = embedded_with_filter("[filter]\nextra_key_segments = [\"passphrase\"]\nextra_keys = [\"X-Auth-Key\"]\n");
    assert_ne!(toml, crate::config::EMBEDDED_TOML);
    let config = Config::from_toml(&toml, crate::config::BUILTIN_MODULES).unwrap();
    assert_eq!(config.filter.extra_key_segments, ["passphrase"]);

    let without = embedded_with_filter("");
    assert_eq!(
        Config::from_toml(&without, crate::config::BUILTIN_MODULES)
            .unwrap()
            .filter,
        FilterConfig::default()
    );

    let toml = embedded_with_filter(
        "[filter]\nextra_key_segments = [\"two_words\", \"\", \"ok\", \"ok\"]\nextra_keys = [\"a b\", \"--\"]\n",
    );
    let errors = Config::from_toml(&toml, crate::config::BUILTIN_MODULES).unwrap_err();
    let paths: Vec<&str> = errors.errors.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "filter.extra_key_segments[0]",
            "filter.extra_key_segments[1]",
            "filter.extra_key_segments[3]",
            "filter.extra_keys[0]",
            "filter.extra_keys[1]",
        ]
    );

    let toml = embedded_with_filter("[filter]\nremove_defaults = true\n");
    assert!(Config::from_toml(&toml, crate::config::BUILTIN_MODULES).is_err());
}
