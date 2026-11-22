//! What the Windows notifications say and which buttons they carry, decided here so that it is tested under Linux.
//! The application only turns a [`ToastContent`] into WinRT objects, through the DOM of an `XmlDocument`: every
//! text below is set as an attribute or as inner text, never pasted into XML.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use mcp_proxy_core::config::SessionId;
use mcp_proxy_core::confirm::ConfirmationRequest;
use serde_json::Value;

use super::pending::ConfirmationId;

/// Application User Model ID of MCP Proxy (S20). Registered under the current user's classes (`super::aumid`) and
/// passed explicitly to every toast.
pub const AUMID: &str = "McpProxy.Desktop";

/// Groups of the toasts, so that one kind can be removed without touching the others.
pub const GROUP_CONFIRMATION: &str = "confirmation";
pub const GROUP_LOGIN: &str = "login";
pub const GROUP_INFO: &str = "info";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastScenario {
    Default,
    /// Pre-expanded, and kept on screen until the user acts on it (needs at least one button).
    Reminder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastButton {
    pub label: String,
    /// Arguments handed back on activation, see [`ToastAction`].
    pub action: String,
}

/// One toast. Every text is already filtered and fit to be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastContent {
    /// Unique among the toasts of the process, short enough for every Windows version (16 characters).
    pub tag: String,
    pub group: &'static str,
    pub title: String,
    /// At most two lines of body text.
    pub lines: Vec<String>,
    pub buttons: Vec<ToastButton>,
    /// Arguments of a click on the body of the toast.
    pub launch: String,
    pub scenario: ToastScenario,
    /// When Windows must remove it if it is still there: the deadline of a confirmation.
    pub deadline: Option<Instant>,
}

/// What the activation of a toast asks for, read back from its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastAction {
    Approve(ConfirmationId),
    Refuse(ConfirmationId),
    /// « Voir le détail », or a click on the body of a confirmation.
    Detail(ConfirmationId),
    /// A click on the login notification: the login window of the session comes to the front.
    Login(SessionId),
    /// A click on an information: the main window opens.
    Open,
}

const APPROVE: &str = "confirm:approve:";
const REFUSE: &str = "confirm:refuse:";
const DETAIL: &str = "confirm:detail:";
const LOGIN: &str = "login:";
const OPEN: &str = "open";

impl ToastAction {
    pub fn arguments(&self) -> String {
        match self {
            Self::Approve(id) => format!("{APPROVE}{id}"),
            Self::Refuse(id) => format!("{REFUSE}{id}"),
            Self::Detail(id) => format!("{DETAIL}{id}"),
            Self::Login(session) => format!("{LOGIN}{session}"),
            Self::Open => OPEN.to_owned(),
        }
    }

    /// `None` for anything this application did not write.
    pub fn parse(arguments: &str) -> Option<Self> {
        let id = |rest: &str| rest.parse::<u64>().ok().map(ConfirmationId::from_raw);
        if let Some(rest) = arguments.strip_prefix(APPROVE) {
            id(rest).map(Self::Approve)
        } else if let Some(rest) = arguments.strip_prefix(REFUSE) {
            id(rest).map(Self::Refuse)
        } else if let Some(rest) = arguments.strip_prefix(DETAIL) {
            id(rest).map(Self::Detail)
        } else if let Some(rest) = arguments.strip_prefix(LOGIN) {
            SessionId::new(rest).ok().map(Self::Login)
        } else if arguments == OPEN {
            Some(Self::Open)
        } else {
            None
        }
    }
}

/// Why Windows removed a toast (`ToastDismissalReason`), `Unknown` when it could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DismissReason {
    UserCanceled,
    ApplicationHidden,
    TimedOut,
    Unknown,
}

impl DismissReason {
    /// From the value of `ToastDismissalReason` (0, 1, 2).
    pub fn from_winrt(value: Option<i32>) -> Self {
        match value {
            Some(0) => Self::UserCanceled,
            Some(1) => Self::ApplicationHidden,
            Some(2) => Self::TimedOut,
            _ => Self::Unknown,
        }
    }
}

/// Whether toasts can be seen right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastState {
    Available,
    /// French, for the log: why the detail window opens instead.
    Unavailable(String),
}

impl ToastState {
    /// From `ToastNotifier::Setting` (`Enabled` = 0) and `ToastNotificationManagerForUser::NotificationMode`
    /// (`Unrestricted` = 0); `None` when the value could not be read. Anything but both enabled and unrestricted
    /// means that a toast may not be seen: the detail window is used.
    pub fn from_winrt(setting: Option<i32>, mode: Option<i32>) -> Self {
        let reason = match (setting, mode) {
            (Some(0), Some(0)) => return Self::Available,
            (None, _) => "état des notifications illisible",
            (Some(1), _) => "notifications désactivées pour MCP Proxy",
            (Some(2), _) => "notifications désactivées pour l'utilisateur",
            (Some(3), _) => "notifications désactivées par une stratégie",
            (Some(_), _) if setting != Some(0) => "notifications désactivées",
            (_, None) => "mode des notifications illisible",
            (_, Some(1)) => "mode « Ne pas déranger » (prioritaires seulement)",
            (_, Some(2)) => "mode « Ne pas déranger » (alarmes seulement)",
            (_, Some(_)) => "mode des notifications restreint",
        };
        Self::Unavailable(reason.to_owned())
    }
}

/// How a confirmation is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presentation {
    /// A toast; with « Autoriser une fois » only when everything fits in it.
    Toast {
        approve: bool,
    },
    DetailWindow,
}

/// Toasts available: a toast, whose approval button needs the whole request to fit. Otherwise the detail window,
/// so that nothing waits unseen.
pub fn presentation(state: &ToastState, fits: bool) -> Presentation {
    match state {
        ToastState::Available => Presentation::Toast { approve: fits },
        ToastState::Unavailable(_) => Presentation::DetailWindow,
    }
}

/// Width budget of one line of a toast, in units: 1 for a narrow character, 2 for a regular one, 3 for a wide one.
/// About 30 regular characters, while a toast shows about 40: the margin covers the font and the scale.
pub const LINE_UNITS: usize = 60;
pub const TITLE_LINES: usize = 2;
pub const CLIENT_LINES: usize = 1;
pub const ARGUMENT_LINES: usize = 3;

/// Characters a toast shows as they are: printable ASCII and the Latin-1 letters and signs, without the
/// no-break and soft hyphen spaces. Anything else (line breaks, control, invisible or bidirectional characters,
/// other scripts) could be shown differently from what it is: such a request goes to the detail window.
fn plain(c: char) -> bool {
    matches!(c, ' '..='~') || (matches!(c, '\u{a1}'..='\u{ff}') && c != '\u{ad}')
}

fn units(c: char) -> usize {
    match c {
        ' ' | 'i' | 'l' | 'j' | 't' | 'f' | 'r' | 'I' | '!' | '.' | ',' | ':' | ';' | '\'' | '|' | '(' | ')' | '['
        | ']' => 1,
        'a'..='z' | '0'..='9' if c != 'm' && c != 'w' => 2,
        '"' | '-' | '_' | '/' | '\\' | '{' | '}' | '<' | '>' | '=' | '+' | '*' | '^' | '~' | '`' | '?' | '$' => 2,
        _ => 3,
    }
}

/// Lines taken by `text` wrapped greedily on spaces at [`LINE_UNITS`], a word wider than a line being cut
/// anywhere. `None` when a character is not [`plain`].
pub fn wrapped_lines(text: &str) -> Option<usize> {
    if !text.chars().all(plain) {
        return None;
    }
    let mut lines = 1;
    let mut used = 0;
    for word in text.split(' ') {
        let width: usize = word.chars().map(units).sum();
        let needed = if used == 0 { width } else { width + 1 };
        if used + needed <= LINE_UNITS {
            used += needed;
            continue;
        }
        if used > 0 {
            lines += 1;
            used = 0;
        }
        for c in word.chars() {
            let w = units(c);
            if used + w > LINE_UNITS {
                lines += 1;
                used = 0;
            }
            used += w;
        }
    }
    Some(lines)
}

fn fits(text: &str, max_lines: usize) -> bool {
    wrapped_lines(text).is_some_and(|lines| lines <= max_lines)
}

/// Every key and string of `value` is [`plain`].
fn plain_json(value: &Value) -> bool {
    match value {
        Value::String(s) => s.chars().all(plain),
        Value::Array(items) => items.iter().all(plain_json),
        Value::Object(map) => map.iter().all(|(k, v)| k.chars().all(plain) && plain_json(v)),
        Value::Null | Value::Bool(_) | Value::Number(_) => true,
    }
}

/// The arguments on one line of the toast: `index: "logs", force: true`, values in compact JSON.
pub fn arguments_line(arguments: &Value) -> String {
    match arguments {
        Value::Object(map) if map.is_empty() => "Aucun argument".to_owned(),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    }
}

fn title(request: &ConfirmationRequest) -> String {
    format!("Confirmation : {}", request.path)
}

fn client_line(client: &str) -> String {
    if client.is_empty() {
        "Demandé par un client LLM".to_owned()
    } else {
        format!("Demandé par {client}")
    }
}

/// Whether everything the user approves can be read on the toast: path, client and every argument, whole. An
/// argument that would be cut, a multi-line value or a character that is not shown as it is: no.
pub fn fits_in_toast(request: &ConfirmationRequest) -> bool {
    plain_json(&request.arguments)
        && fits(&title(request), TITLE_LINES)
        && fits(&client_line(&request.client), CLIENT_LINES)
        && fits(&arguments_line(&request.arguments), ARGUMENT_LINES)
}

/// Tag of the toast of `id`, to remove it.
pub fn confirmation_tag(id: ConfirmationId) -> String {
    format!("c{id}")
}

/// The confirmation toast: « Refuser » and « Autoriser une fois » when `approve`, « Refuser » and « Voir le détail »
/// otherwise. Kept on screen until the decision (reminder); a click on its body opens the detail window. When the
/// request does not fit, the lines may be cut by Windows: they are only a hint, the approval is in the window.
pub fn confirmation_toast(id: ConfirmationId, request: &ConfirmationRequest, approve: bool) -> ToastContent {
    let second = if approve {
        ToastButton {
            label: "Autoriser une fois".to_owned(),
            action: ToastAction::Approve(id).arguments(),
        }
    } else {
        ToastButton {
            label: "Voir le détail".to_owned(),
            action: ToastAction::Detail(id).arguments(),
        }
    };
    let arguments = if approve {
        visible(&arguments_line(&request.arguments))
    } else {
        "Arguments trop longs pour cette notification : voir le détail.".to_owned()
    };
    ToastContent {
        tag: confirmation_tag(id),
        group: GROUP_CONFIRMATION,
        title: visible(&title(request)),
        lines: vec![visible(&client_line(&request.client)), arguments],
        buttons: vec![
            ToastButton {
                label: "Refuser".to_owned(),
                action: ToastAction::Refuse(id).arguments(),
            },
            second,
        ],
        launch: ToastAction::Detail(id).arguments(),
        scenario: ToastScenario::Reminder,
        deadline: Some(request.deadline),
    }
}

/// « Connexion demandée » : `{client} attend une connexion à {session}`. `client` is the filtered `clientInfo.name`,
/// empty when unknown. A click brings the login window of the session to the front.
pub fn login_toast(number: u64, session: &SessionId, session_label: &str, client: &str) -> ToastContent {
    let who = if client.is_empty() {
        "Un client LLM".to_owned()
    } else {
        visible(client)
    };
    ToastContent {
        tag: format!("l{number}"),
        group: GROUP_LOGIN,
        title: "Connexion demandée".to_owned(),
        lines: vec![
            format!("{who} attend une connexion à {}", visible(session_label)),
            "Cliquez pour afficher la fenêtre de connexion.".to_owned(),
        ],
        buttons: Vec::new(),
        launch: ToastAction::Login(session.clone()).arguments(),
        scenario: ToastScenario::Default,
        deadline: None,
    }
}

/// The information notifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Information {
    /// A new version of MCP Proxy is ready (spec 020).
    UpdateReady { version: String },
    /// The tools of an MCP changed (spec 008).
    McpUpdated { mcp: String, environment: String },
    /// The settings file was unreadable and replaced by the strictest settings.
    SettingsReset,
}

pub fn information_toast(number: u64, information: &Information) -> ToastContent {
    let (title, line) = match information {
        Information::UpdateReady { version } => (
            "Mise à jour prête".to_owned(),
            format!("MCP Proxy {} sera utilisé au prochain démarrage.", visible(version)),
        ),
        Information::McpUpdated { mcp, environment } => (
            "MCP mis à jour".to_owned(),
            format!("Les outils de {} ({}) ont changé.", visible(mcp), visible(environment)),
        ),
        Information::SettingsReset => (
            "Réglages réinitialisés".to_owned(),
            "Vos réglages étaient illisibles : les plus stricts s'appliquent. Choisissez de nouveau votre profil."
                .to_owned(),
        ),
    };
    ToastContent {
        tag: format!("i{number}"),
        group: GROUP_INFO,
        title,
        lines: vec![line],
        buttons: Vec::new(),
        launch: ToastAction::Open.arguments(),
        scenario: ToastScenario::Default,
        deadline: None,
    }
}

/// `text` with every control, invisible or bidirectional formatting character written as `\u{…}`, so that what
/// is shown is what was asked. Other characters are kept.
pub fn visible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let hidden = c.is_control()
            || matches!(
                c,
                '\u{ad}'
                    | '\u{34f}'
                    | '\u{61c}'
                    | '\u{115f}'
                    | '\u{1160}'
                    | '\u{17b4}'
                    | '\u{17b5}'
                    | '\u{180b}'..='\u{180f}'
                    | '\u{200b}'..='\u{200f}'
                    | '\u{2028}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}'
                    | '\u{3164}'
                    | '\u{fe00}'..='\u{fe0f}'
                    | '\u{feff}'
                    | '\u{ffa0}'
                    | '\u{fff0}'..='\u{fffb}'
                    | '\u{e0000}'..='\u{e0fff}'
            );
        if hidden {
            out.push_str(&format!("\\u{{{:04x}}}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// Offset between 1601-01-01 and 1970-01-01, in 100 ns ticks.
const WINDOWS_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// `at` as a WinRT `DateTime` (`UniversalTime`, 100 ns ticks since 1601-01-01 UTC). `None` before 1970 or out of
/// range.
pub fn windows_ticks(at: SystemTime) -> Option<i64> {
    let since = at.duration_since(UNIX_EPOCH).ok()?;
    let ticks = i64::try_from(since.as_nanos() / 100).ok()?;
    ticks.checked_add(WINDOWS_EPOCH_TICKS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::time::Duration;

    use mcp_proxy_core::config::{EnvId, McpId, ToolName};
    use serde_json::json;

    use super::*;

    fn request(arguments: Value) -> ConfirmationRequest {
        ConfirmationRequest {
            env: EnvId::new("prod").unwrap(),
            mcp: McpId::new("kibana-logs").unwrap(),
            tool: ToolName::new("delete_index").unwrap(),
            path: "PROD / kibana-logs / delete_index".to_owned(),
            client: "Claude Desktop".to_owned(),
            arguments,
            explanation: "outil destructif en PROD : confirmation demandée".to_owned(),
            deadline: Instant::now(),
        }
    }

    fn id(raw: u64) -> ConfirmationId {
        ConfirmationId::from_raw(raw)
    }

    // Exigence : EX-013-02
    #[test]
    fn short_arguments_fit() {
        assert!(fits_in_toast(&request(json!({}))));
        assert!(fits_in_toast(&request(
            json!({ "index": "logs-2026.11", "force": true })
        )));
        assert!(fits_in_toast(&request(
            json!({ "query": "état = clôturé", "limit": 10 })
        )));
        assert!(fits_in_toast(&request(json!({ "ids": [1, 2, 3], "dry_run": false }))));
        assert_eq!(
            arguments_line(&json!({ "index": "logs-2026.11", "force": true })),
            "force: true, index: \"logs-2026.11\""
        );
        assert_eq!(arguments_line(&json!({})), "Aucun argument");
    }

    // Exigence : EX-013-02
    #[test]
    fn long_arguments_do_not_fit() {
        let long = "x".repeat(200);
        assert!(!fits_in_toast(&request(json!({ "query": long }))));
        let many: serde_json::Map<String, Value> = (0..12).map(|i| (format!("champ{i}"), json!(i))).collect();
        assert!(!fits_in_toast(&request(Value::Object(many))));
        // Three lines are allowed, a fourth is not: 30 regular characters per line.
        let words = |n: usize| vec!["abcdefghi"; n].join(" ");
        assert_eq!(wrapped_lines(&words(3)), Some(1), "29 characters, 1 line");
        assert_eq!(wrapped_lines(&words(4)), Some(2));
        assert_eq!(wrapped_lines(&"a".repeat(31)), Some(2), "a long word is cut");
        assert_eq!(wrapped_lines(&"W".repeat(21)), Some(2), "wide characters count more");
        assert!(fits_in_toast(&request(json!({ "q": words(8) }))));
        assert!(!fits_in_toast(&request(json!({ "q": words(10) }))));
        // The path and the client must fit too.
        let mut wide_path = request(json!({}));
        wide_path.path = format!("PROD / kibana-logs / {}", "a".repeat(60));
        assert!(!fits_in_toast(&wide_path));
        let mut wide_client = request(json!({}));
        wide_client.client = "c".repeat(40);
        assert!(!fits_in_toast(&wide_client));
    }

    // Exigence : EX-013-02
    #[test]
    fn multi_line_or_hidden_characters_do_not_fit() {
        assert!(!fits_in_toast(&request(json!({ "body": "ligne 1\nligne 2" }))));
        assert!(!fits_in_toast(&request(json!({ "body": "a\r\nb" }))));
        assert!(!fits_in_toast(&request(json!({ "nested": { "body": "a\tb" } }))));
        assert!(!fits_in_toast(&request(json!({ "line\nkey": 1 }))));
        // Bidirectional override, zero-width space, soft hyphen, another script.
        assert!(!fits_in_toast(&request(json!({ "name": "abc\u{202e}fdp.exe" }))));
        assert!(!fits_in_toast(&request(json!({ "name": "a\u{200b}b" }))));
        assert!(!fits_in_toast(&request(json!({ "name": "a\u{ad}b" }))));
        assert!(!fits_in_toast(&request(json!({ "name": "ключ" }))));
        assert_eq!(wrapped_lines("a\nb"), None);
    }

    // Exigence : EX-013-02
    #[test]
    fn buttons_follow_the_rule() {
        let short = request(json!({ "index": "logs" }));
        let toast = confirmation_toast(id(7), &short, true);
        let labels: Vec<&str> = toast.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["Refuser", "Autoriser une fois"]);
        assert_eq!(toast.buttons[1].action, "confirm:approve:7");
        assert_eq!(toast.scenario, ToastScenario::Reminder);
        assert_eq!(toast.launch, "confirm:detail:7", "a click on the body never approves");

        let long = request(json!({ "query": "x".repeat(300) }));
        let toast = confirmation_toast(id(8), &long, false);
        let labels: Vec<&str> = toast.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["Refuser", "Voir le détail"]);
        assert!(toast.buttons.iter().all(|b| !b.action.starts_with("confirm:approve")));
        assert!(!toast.lines.concat().contains(&"x".repeat(300)));

        assert_eq!(
            presentation(&ToastState::Available, true),
            Presentation::Toast { approve: true }
        );
        assert_eq!(
            presentation(&ToastState::Available, false),
            Presentation::Toast { approve: false }
        );
        let off = ToastState::Unavailable("x".to_owned());
        assert_eq!(presentation(&off, true), Presentation::DetailWindow);
    }

    // Exigence : EX-013-02
    #[test]
    fn toast_state_and_dismissal_from_winrt() {
        assert_eq!(ToastState::from_winrt(Some(0), Some(0)), ToastState::Available);
        for (setting, mode) in [
            (Some(1), Some(0)),
            (Some(2), Some(0)),
            (Some(3), Some(0)),
            (Some(4), Some(0)),
            (None, Some(0)),
            (Some(0), Some(1)),
            (Some(0), Some(2)),
            (Some(0), Some(9)),
            (Some(0), None),
        ] {
            assert!(
                matches!(ToastState::from_winrt(setting, mode), ToastState::Unavailable(_)),
                "{setting:?} {mode:?}"
            );
        }
        assert_eq!(
            ToastState::from_winrt(Some(0), Some(1)),
            ToastState::Unavailable("mode « Ne pas déranger » (prioritaires seulement)".to_owned())
        );
        assert_eq!(DismissReason::from_winrt(Some(0)), DismissReason::UserCanceled);
        assert_eq!(DismissReason::from_winrt(Some(1)), DismissReason::ApplicationHidden);
        assert_eq!(DismissReason::from_winrt(Some(2)), DismissReason::TimedOut);
        assert_eq!(DismissReason::from_winrt(Some(7)), DismissReason::Unknown);
        assert_eq!(DismissReason::from_winrt(None), DismissReason::Unknown);
    }

    #[test]
    fn actions_round_trip_and_reject_anything_else() {
        let session = SessionId::new("sso-prod").unwrap();
        for action in [
            ToastAction::Approve(id(3)),
            ToastAction::Refuse(id(3)),
            ToastAction::Detail(id(18_446_744_073_709_551_615)),
            ToastAction::Login(session.clone()),
            ToastAction::Open,
        ] {
            assert_eq!(ToastAction::parse(&action.arguments()), Some(action));
        }
        for junk in [
            "",
            "confirm:approve:",
            "confirm:approve:-1",
            "confirm:approve:1 ",
            "confirm:approve:0x1",
            "confirm:accept:1",
            "login:",
            "login:../x",
            "open ",
            "approve",
        ] {
            assert_eq!(ToastAction::parse(junk), None, "{junk:?}");
        }
    }

    // Exigence : EX-013-03
    #[test]
    fn texts_hide_nothing() {
        assert_eq!(visible("abc\u{202e}fdp.exe"), "abc\\u{202e}fdp.exe");
        assert_eq!(visible("a\u{200b}b\u{7}c"), "a\\u{200b}b\\u{0007}c");
        assert_eq!(visible("état ✓"), "état ✓");
        let session = SessionId::new("sso-prod").unwrap();
        let toast = login_toast(4, &session, "SSO PROD", "Claude Desktop");
        assert_eq!(toast.title, "Connexion demandée");
        assert_eq!(toast.lines[0], "Claude Desktop attend une connexion à SSO PROD");
        assert_eq!(toast.launch, "login:sso-prod");
        assert!(toast.buttons.is_empty());
        let anonymous = login_toast(5, &session, "SSO PROD", "");
        assert_eq!(anonymous.lines[0], "Un client LLM attend une connexion à SSO PROD");
        assert_ne!(toast.tag, anonymous.tag);

        let info = information_toast(
            1,
            &Information::McpUpdated {
                mcp: "Kibana".to_owned(),
                environment: "PROD".to_owned(),
            },
        );
        assert_eq!(info.title, "MCP mis à jour");
        assert_eq!(info.lines, ["Les outils de Kibana (PROD) ont changé."]);
        assert_eq!(info.launch, "open");
        assert_eq!(
            information_toast(2, &Information::SettingsReset).title,
            "Réglages réinitialisés"
        );
        assert_eq!(
            information_toast(
                3,
                &Information::UpdateReady {
                    version: "1.2.0".to_owned()
                }
            )
            .lines,
            ["MCP Proxy 1.2.0 sera utilisé au prochain démarrage."]
        );
        for content in [&toast, &info] {
            assert!(content.tag.len() <= 16);
        }
    }

    #[test]
    fn windows_time() {
        assert_eq!(windows_ticks(UNIX_EPOCH), Some(WINDOWS_EPOCH_TICKS));
        // 2026-11-22T14:03:07Z
        let at = UNIX_EPOCH + Duration::from_secs(1_795_356_187);
        assert_eq!(windows_ticks(at), Some(WINDOWS_EPOCH_TICKS + 17_953_561_870_000_000));
        assert_eq!(windows_ticks(UNIX_EPOCH - Duration::from_secs(1)), None);
    }
}
