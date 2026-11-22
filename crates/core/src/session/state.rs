//! State of a session, as the interface shows it.

use std::time::{Duration, SystemTime};

use crate::trace::time::utc;

/// A session is considered expired this long before the expiry of its credential, so that a call started just
/// before does not reach the server with a credential about to expire.
pub const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    NeverConnected,
    /// Usable until `until`, margin already taken off.
    Active {
        until: SystemTime,
    },
    /// Expired, refused by a server, or disconnected: the secret is wiped.
    Expired,
    /// A login was asked for at `since` and is not finished.
    LoginRequested {
        since: SystemTime,
    },
}

impl SessionState {
    /// French label: « jamais connectée », « active jusqu'à HH:MM », « expirée », « connexion demandée ».
    /// `utc_offset` is the local offset from UTC at that time, given by the application (east positive).
    pub fn label(&self, utc_offset: i32) -> String {
        match self {
            Self::NeverConnected => "jamais connectée".to_owned(),
            Self::Active { until } => {
                let local = shift(*until, utc_offset);
                let t = utc(local);
                format!("active jusqu'à {:02}:{:02}", t.hour, t.minute)
            }
            Self::Expired => "expirée".to_owned(),
            Self::LoginRequested { .. } => "connexion demandée".to_owned(),
        }
    }

    pub fn is_active_at(&self, now: SystemTime) -> bool {
        matches!(self, Self::Active { until } if now < *until)
    }
}

fn shift(at: SystemTime, offset_secs: i32) -> SystemTime {
    let by = Duration::from_secs(u64::from(offset_secs.unsigned_abs()));
    let shifted = if offset_secs >= 0 {
        at.checked_add(by)
    } else {
        at.checked_sub(by)
    };
    shifted.unwrap_or(at)
}

/// Why a state changed, for the interface and the notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionChange {
    /// A call or the user asked for a login.
    LoginRequested,
    /// The login succeeded: the instances using the session may now be reloaded.
    LoggedIn,
    /// The user closed the window or cancelled the input.
    LoginCancelled,
    /// The login could not be completed (identifiers refused three times, probe unreachable, window error).
    LoginFailed,
    /// The expiry was reached; the secret is wiped.
    Expired,
    /// A server refused the credential; the secret is wiped.
    Rejected,
    /// The user disconnected the session; the secret is wiped.
    Disconnected,
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use super::*;

    #[test]
    fn labels() {
        // 2026-11-22T14:03:07Z
        let until = UNIX_EPOCH + Duration::from_secs(1_795_356_187);
        assert_eq!(SessionState::NeverConnected.label(0), "jamais connectée");
        assert_eq!(SessionState::Active { until }.label(0), "active jusqu'à 14:03");
        assert_eq!(SessionState::Active { until }.label(3600), "active jusqu'à 15:03");
        assert_eq!(SessionState::Active { until }.label(-15 * 3600), "active jusqu'à 23:03");
        assert_eq!(SessionState::Expired.label(0), "expirée");
        assert_eq!(
            SessionState::LoginRequested { since: until }.label(0),
            "connexion demandée"
        );
        assert!(SessionState::Active { until }.is_active_at(until - Duration::from_secs(1)));
        assert!(!SessionState::Active { until }.is_active_at(until));
        assert!(!SessionState::Expired.is_active_at(UNIX_EPOCH));
    }
}
