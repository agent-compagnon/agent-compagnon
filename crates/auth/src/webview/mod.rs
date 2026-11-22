//! What the WebView2 login windows of the application (spec 012) decide, written here so that it is tested under
//! Linux: which navigations are allowed, which remote URLs the deposit capability covers, the capture script and
//! its configuration, the checks of every deposit, the answers of the Basic window, the WebView2 runtime version
//! and the browser arguments. The application only wires these to Tauri.
//!
//! The capture script observes `fetch` and `XMLHttpRequest` in the page and hands a candidate header value to the
//! deposit command, the only command a remote page can reach (INV-8 and its exception). Nothing it hands over is
//! trusted: [`LoginWindows::deposit`] checks the calling window, the host of the calling page, the size, then the
//! checks of [`crate::capture`], and keeps only the first valid token of a window.

mod basic;
mod deposit;
mod front;
mod navigation;
mod profile;
mod script;

pub use basic::{BasicPrompts, MalformedAnswer, parse_basic_answer};
pub use deposit::{DepositRejection, LoginWindows};
pub use front::SessionWindows;
pub use navigation::{
    app_page_allowed, cookie_domain_matches, destination_for_log, navigation_allowed, remote_url_patterns,
};
pub use profile::{
    LoginProfile, MIN_RUNTIME_VERSION, OS_SSO_FEATURE, WRY_DEFAULT_ARGS, browser_args, check_runtime_version,
    reset_profile_dir,
};
pub use script::{BASIC_COMMAND, CAPTURE_SOURCE, DEPOSIT_COMMAND, basic_prompt_script, capture_script, js_data};
