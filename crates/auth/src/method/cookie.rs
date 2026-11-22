//! `cookie`: session cookies read from the cookie manager once the login page reached its success URL.

use std::time::SystemTime;

use async_trait::async_trait;
use mcp_proxy_core::config::{AuthConfig, Session, SessionId, UrlPattern};
use mcp_proxy_core::secrets::Credential;
use mcp_proxy_core::session::EXPIRY_MARGIN;
use url::Url;

use super::{AuthMethod, LoginContext, LoginFailure, Obtained, after_hours, next_event};
use crate::capture::url_matches;
use crate::ui::{CookiePair, LoginEvent, LoginWindowSpec, window_title};

#[derive(Debug, Clone)]
pub struct CookieMethod {
    session: SessionId,
    label: String,
    login_url: Url,
    cookie_url: Url,
    cookie_names: Vec<String>,
    success_url_pattern: UrlPattern,
    allowed_navigation: Vec<String>,
    ttl_hours: u32,
}

/// Why the cookies read after a success navigation were not retained. Names only, never values.
enum Missing {
    Absent(String),
    Expiring(String),
}

impl CookieMethod {
    /// `None` if the session does not use `cookie`.
    pub fn from_config(session: &Session) -> Option<Self> {
        let AuthConfig::Cookie {
            login_url,
            cookie_url,
            cookie_names,
            success_url_pattern,
            allowed_navigation,
            ttl_hours,
        } = &session.auth
        else {
            return None;
        };
        Some(Self {
            session: session.id.clone(),
            label: session.label.clone(),
            login_url: login_url.clone(),
            cookie_url: cookie_url.clone(),
            cookie_names: cookie_names.clone(),
            success_url_pattern: success_url_pattern.clone(),
            allowed_navigation: allowed_navigation.clone(),
            ttl_hours: *ttl_hours,
        })
    }

    /// The window this method opens.
    pub fn window_spec(&self) -> LoginWindowSpec {
        LoginWindowSpec {
            session: self.session.clone(),
            title: window_title(&self.label),
            login_url: self.login_url.clone(),
            allowed_navigation: self.allowed_navigation.clone(),
            capture: None,
        }
    }

    /// Every expected cookie, in the configured order, with the earliest expiry among them. Only the expected
    /// cookies are kept. When the manager gives a name twice, the first one wins.
    fn select(&self, cookies: Vec<CookiePair>, now: SystemTime) -> Result<Obtained, Missing> {
        let mut cookies: Vec<Option<CookiePair>> = cookies.into_iter().map(Some).collect();
        let usable_after = now.checked_add(EXPIRY_MARGIN).unwrap_or(now);
        let mut expires_at = after_hours(now, self.ttl_hours);
        let mut pairs = Vec::with_capacity(self.cookie_names.len());
        for name in &self.cookie_names {
            let cookie = cookies
                .iter_mut()
                .find(|c| c.as_ref().is_some_and(|c| &c.name == name))
                .and_then(Option::take)
                .ok_or_else(|| Missing::Absent(name.clone()))?;
            if let Some(expires) = cookie.expires {
                if expires <= usable_after {
                    return Err(Missing::Expiring(name.clone()));
                }
                expires_at = expires_at.min(expires);
            }
            pairs.push((cookie.name, cookie.value));
        }
        Ok(Obtained {
            credential: Credential::Cookies { pairs },
            expires_at,
        })
    }
}

#[async_trait]
impl AuthMethod for CookieMethod {
    fn site_hosts(&self) -> Vec<String> {
        let mut hosts = self.allowed_navigation.clone();
        if let Some(host) = self.cookie_url.host_str()
            && !hosts.iter().any(|h| h == host)
        {
            hosts.push(host.to_owned());
        }
        hosts
    }

    async fn login(&self, context: LoginContext<'_>) -> Result<Obtained, LoginFailure> {
        let mut window = context.ui.login_window(self.window_spec()).await?;
        loop {
            let url = match next_event(&mut window, context.cancel).await {
                None | Some(LoginEvent::Closed) => {
                    window.close().await;
                    return Err(LoginFailure::Cancelled);
                }
                Some(LoginEvent::Navigated { url }) => url,
                // A cookie session ignores deposits: nothing is captured from headers.
                Some(LoginEvent::HeaderCaptured { .. }) => continue,
            };
            if !url_matches(&self.success_url_pattern, &url) {
                continue;
            }
            let cookies = match window.cookies(&self.cookie_url).await {
                Ok(cookies) => cookies,
                Err(error) => {
                    tracing::warn!(session = self.session.as_str(), %error, "cookies de session illisibles");
                    continue;
                }
            };
            match self.select(cookies, context.clock.now()) {
                Ok(obtained) => {
                    window.close().await;
                    return Ok(obtained);
                }
                Err(Missing::Absent(name)) => tracing::info!(
                    session = self.session.as_str(),
                    cookie = name.as_str(),
                    "cookie de session attendu absent : la connexion continue"
                ),
                Err(Missing::Expiring(name)) => tracing::info!(
                    session = self.session.as_str(),
                    cookie = name.as_str(),
                    "cookie de session expiré ou sur le point de l'être : la connexion continue"
                ),
            }
        }
    }
}
