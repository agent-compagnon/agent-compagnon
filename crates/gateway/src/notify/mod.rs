//! Windows notifications and confirmations outside the chat (spec 013), decided here and tested under Linux: the
//! identity of the application for the toasts, the texts and buttons of every toast, the rule « fits in the toast »,
//! the choice between a toast and the detail window, the registry of waiting confirmations and the broker that the
//! call chain asks. The application only shows what it is given and passes the answers on.
//!
//! Only an explicit approval lets a call through (INV-5): a refusal, the detail window closed, a dismissed or
//! ignored toast, an unknown request, a second answer, a display error or the application stopping never do.

mod aumid;
mod broker;
mod detail;
mod pending;
mod toast;

pub use aumid::{AumidRegistration, DISPLAY_NAME, aumid_registration, write_icon};
pub use broker::{ConfirmationPresenter, NotificationBroker, PresentError, UnavailablePresenter};
pub use detail::{CONFIRM_COMMAND, DetailView, confirmation_of_label, detail_label, detail_script};
pub use pending::{Answer, AnswerSource, Closed, ConfirmationId, PendingConfirmations, ToastFollowUp, Waiting};
pub use toast::{
    ARGUMENT_LINES, AUMID, CLIENT_LINES, DismissReason, GROUP_CONFIRMATION, GROUP_INFO, GROUP_LOGIN, Information,
    LINE_UNITS, Presentation, TITLE_LINES, ToastAction, ToastButton, ToastContent, ToastScenario, ToastState,
    arguments_line, confirmation_tag, confirmation_toast, fits_in_toast, information_toast, login_toast, presentation,
    visible, windows_ticks, wrapped_lines,
};
