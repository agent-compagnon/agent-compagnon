//! Which window a click on the login notification brings to the front: the login or Basic window open right now
//! for the session (spec 013). The notification is shown right before the window opens, so the window is looked up
//! at the click.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use mcp_proxy_core::config::SessionId;

#[derive(Debug, Default)]
pub struct SessionWindows {
    labels: Mutex<HashMap<SessionId, String>>,
}

impl SessionWindows {
    fn lock(&self) -> MutexGuard<'_, HashMap<SessionId, String>> {
        self.labels.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The window `label` is now the one of `session`.
    pub fn opened(&self, session: &SessionId, label: &str) {
        self.lock().insert(session.clone(), label.to_owned());
    }

    /// The window `label` is gone.
    pub fn closed(&self, label: &str) {
        self.lock().retain(|_, l| l != label);
    }

    /// The window of `session`, if one is open.
    pub fn of(&self, session: &SessionId) -> Option<String> {
        self.lock().get(session).cloned()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_window_of_a_session_is_the_last_one_opened() {
        let windows = SessionWindows::default();
        let prod = SessionId::new("sso-prod").unwrap();
        let tools = SessionId::new("sso-tools").unwrap();
        assert_eq!(windows.of(&prod), None);
        windows.opened(&prod, "login-1");
        windows.opened(&tools, "basic-2");
        assert_eq!(windows.of(&prod).as_deref(), Some("login-1"));
        windows.opened(&prod, "login-3");
        windows.closed("login-1");
        assert_eq!(
            windows.of(&prod).as_deref(),
            Some("login-3"),
            "an older window closing changes nothing"
        );
        windows.closed("login-3");
        assert_eq!(windows.of(&prod), None);
        assert_eq!(windows.of(&tools).as_deref(), Some("basic-2"));
    }
}
