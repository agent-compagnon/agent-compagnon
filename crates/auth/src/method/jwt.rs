//! `jwt_header`: a JWT captured in the headers of the requests of the login window.

use async_trait::async_trait;
use mcp_proxy_core::config::{AuthConfig, Session, SessionId};
use mcp_proxy_core::secrets::Credential;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::{AuthMethod, LoginContext, LoginFailure, Obtained, after_hours, next_event};
use crate::capture::{CaptureRejection, JwtExpectations, check_jwt};
use crate::ui::{HeaderCapture, LoginEvent, LoginWindowSpec, window_title};

#[derive(Debug, Clone)]
pub struct JwtHeaderMethod {
    session: SessionId,
    label: String,
    login_url: Url,
    capture: HeaderCapture,
    allowed_navigation: Vec<String>,
    max_ttl_hours: u32,
}

impl JwtHeaderMethod {
    /// `None` if the session does not use `jwt_header`.
    pub fn from_config(session: &Session) -> Option<Self> {
        let AuthConfig::JwtHeader {
            login_url,
            capture_url_pattern,
            capture_header,
            capture_scheme,
            expected_audience,
            expected_issuer,
            allowed_navigation,
            max_ttl_hours,
        } = &session.auth
        else {
            return None;
        };
        Some(Self {
            session: session.id.clone(),
            label: session.label.clone(),
            login_url: login_url.clone(),
            capture: HeaderCapture {
                url_pattern: capture_url_pattern.clone(),
                header: capture_header.clone(),
                scheme: capture_scheme.clone(),
                expect: JwtExpectations {
                    audience: expected_audience.clone(),
                    issuer: expected_issuer.clone(),
                },
            },
            allowed_navigation: allowed_navigation.clone(),
            max_ttl_hours: *max_ttl_hours,
        })
    }

    /// The window this method opens.
    pub fn window_spec(&self) -> LoginWindowSpec {
        LoginWindowSpec {
            session: self.session.clone(),
            title: window_title(&self.label),
            login_url: self.login_url.clone(),
            allowed_navigation: self.allowed_navigation.clone(),
            capture: Some(self.capture.clone()),
        }
    }
}

#[async_trait]
impl AuthMethod for JwtHeaderMethod {
    fn site_hosts(&self) -> Vec<String> {
        self.allowed_navigation.clone()
    }

    async fn login(&self, context: LoginContext<'_>) -> Result<Obtained, LoginFailure> {
        let mut window = context.ui.login_window(self.window_spec()).await?;
        loop {
            let event = match next_event(&mut window, context.cancel).await {
                None | Some(LoginEvent::Closed) => {
                    window.close().await;
                    return Err(LoginFailure::Cancelled);
                }
                Some(event) => event,
            };
            let LoginEvent::HeaderCaptured { url, header, value } = event else {
                continue;
            };
            let now = context.clock.now();
            let checked = self
                .capture
                .candidate_token(&url, &header, value.expose_secret())
                .and_then(|token| Ok((token, check_jwt(token, now, self.capture.expectations())?)));
            match checked {
                Ok((token, claims)) => {
                    let token = SecretString::from(token.to_owned());
                    drop(value);
                    window.close().await;
                    let cap = after_hours(now, self.max_ttl_hours);
                    return Ok(Obtained {
                        credential: Credential::Bearer { token },
                        expires_at: claims.expires_at.min(cap),
                    });
                }
                // Neither the value nor the URL (its query may carry anything) is logged.
                Err(reason) => log_ignored(&self.session, reason),
            }
        }
    }
}

fn log_ignored(session: &SessionId, reason: CaptureRejection) {
    tracing::info!(
        session = session.as_str(),
        %reason,
        "valeur candidate de la fenêtre de connexion ignorée"
    );
}
