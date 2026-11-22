//! Authentication methods and on-demand authentication (spec 010).
//!
//! - [`ui`]: what the application provides (login windows, Basic input, notification), implemented with WebView2 by
//!   spec 012 and faked by the testkit;
//! - [`method`]: `jwt_header`, `cookie` and `basic`, behind the extensible [`AuthMethod`] trait;
//! - [`capture`]: checks of every value a login window hands over;
//! - [`OnDemandAuthenticator`]: the `SessionGate` that logs in when a call needs it, always visibly.

mod authenticator;
pub mod capture;
mod clock;
pub mod method;
pub mod probe;
pub mod ui;

pub use authenticator::{AuthenticatorDeps, OnDemandAuthenticator};
pub use clock::TokioWallClock;
pub use method::{AuthMethod, LoginContext, LoginFailure, Obtained, methods_from_config};
pub use probe::{BasicProbe, HttpProbe, ProbeFailure, ProbeOutcome};
pub use ui::{
    AuthUi, AuthUiError, BasicInput, BasicPromptSpec, CookiePair, HeaderCapture, LoginEvent, LoginWindow,
    LoginWindowSpec, UnavailableAuthUi,
};
