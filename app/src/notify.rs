//! Windows notifications through the WinRT projection of the `windows` crate (no COM nor Win32 call, no `unsafe`):
//! the AUMID registered in the registry only, the state of the notifications, and toasts built through the DOM of
//! an `XmlDocument`, so that no text is ever pasted into XML. What a toast says and what its activation means is
//! decided in `mcp_proxy_gateway::notify`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use mcp_proxy_gateway::notify::{
    AUMID, DismissReason, ToastContent, ToastScenario, ToastState, aumid_registration, windows_ticks, write_icon,
};
use mcp_proxy_gateway::runtime::DataDirs;
use windows::Data::Xml::Dom::{XmlDocument, XmlElement};
use windows::Foundation::{DateTime, IReference, PropertyValue, TypedEventHandler};
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastDismissedEventArgs, ToastFailedEventArgs, ToastNotification,
    ToastNotificationManager, ToastNotifier,
};
use windows::core::{HSTRING, IInspectable, Interface};
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const ICON: &[u8] = include_bytes!("../icons/icon.png");

/// What happened to a toast, as Windows reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastEvent {
    /// A button or the body: the arguments it carries.
    Activated(Option<String>),
    Dismissed(DismissReason),
    Failed,
}

/// The toasts of the application. Every method calls WinRT synchronously: call it from a blocking task.
pub struct Toaster {
    /// The AUMID registration at start; without it a toast could be accepted by Windows and never shown.
    registered: Result<(), String>,
    shown: Mutex<HashMap<String, (&'static str, ToastNotification)>>,
    next: AtomicU64,
}

impl std::fmt::Debug for Toaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toaster")
            .field("registered", &self.registered)
            .finish_non_exhaustive()
    }
}

/// Writes the icon and the registry values of the AUMID. Idempotent, current user only.
fn register(dirs: &DataDirs) -> Result<(), String> {
    write_icon(&dirs.notification_icon, ICON).map_err(|e| format!("icône non écrite ({})", e.kind()))?;
    let registration =
        aumid_registration(&dirs.notification_icon).ok_or_else(|| "chemin de l'icône illisible".to_owned())?;
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(&registration.key)
        .map_err(|e| format!("clé du registre inaccessible ({})", e.kind()))?;
    key.set_value("DisplayName", &registration.display_name)
        .and_then(|()| key.set_value("IconUri", &registration.icon_uri))
        .map_err(|e| format!("valeur du registre non écrite ({})", e.kind()))
}

fn notifier() -> windows::core::Result<ToastNotifier> {
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
}

fn element(doc: &XmlDocument, name: &str) -> windows::core::Result<XmlElement> {
    doc.CreateElement(&HSTRING::from(name))
}

fn attribute(element: &XmlElement, name: &str, value: &str) -> windows::core::Result<()> {
    element.SetAttribute(&HSTRING::from(name), &HSTRING::from(value))
}

/// `<toast launch scenario><visual><binding template="ToastGeneric"><text/>…</binding></visual><actions>…`
fn document(content: &ToastContent) -> windows::core::Result<XmlDocument> {
    let doc = XmlDocument::new()?;
    let toast = element(&doc, "toast")?;
    attribute(&toast, "launch", &content.launch)?;
    if content.scenario == ToastScenario::Reminder {
        attribute(&toast, "scenario", "reminder")?;
    }
    let visual = element(&doc, "visual")?;
    let binding = element(&doc, "binding")?;
    attribute(&binding, "template", "ToastGeneric")?;
    for text in std::iter::once(&content.title).chain(&content.lines) {
        let node = element(&doc, "text")?;
        node.SetInnerText(&HSTRING::from(text.as_str()))?;
        binding.AppendChild(&node)?;
    }
    visual.AppendChild(&binding)?;
    toast.AppendChild(&visual)?;
    if !content.buttons.is_empty() {
        let actions = element(&doc, "actions")?;
        for button in &content.buttons {
            let action = element(&doc, "action")?;
            attribute(&action, "content", &button.label)?;
            attribute(&action, "arguments", &button.action)?;
            actions.AppendChild(&action)?;
        }
        toast.AppendChild(&actions)?;
    }
    doc.AppendChild(&toast)?;
    Ok(doc)
}

fn activated_arguments(args: &Option<IInspectable>) -> Option<String> {
    let args = args.as_ref()?.cast::<ToastActivatedEventArgs>().ok()?;
    let arguments = args.Arguments().ok()?;
    (!arguments.is_empty()).then(|| arguments.to_string_lossy())
}

fn dismissal(args: &Option<ToastDismissedEventArgs>) -> DismissReason {
    DismissReason::from_winrt(args.as_ref().and_then(|a| a.Reason().ok()).map(|r| r.0))
}

impl Toaster {
    /// Registers the AUMID and removes the toasts a previous run may have left in the notification centre: their
    /// handlers died with that run.
    pub fn start(dirs: &DataDirs) -> Self {
        let registered = register(dirs);
        if let Err(reason) = &registered {
            tracing::error!(%reason, "identité des notifications non enregistrée : fenêtres à la place des notifications");
        }
        if let Err(error) = ToastNotificationManager::History().and_then(|h| h.ClearWithId(&HSTRING::from(AUMID))) {
            tracing::info!(code = error.code().0, "anciennes notifications non retirées");
        }
        Self {
            registered,
            shown: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        }
    }

    /// No notification at all: every confirmation opens its detail window.
    pub fn unavailable(reason: &str) -> Self {
        Self {
            registered: Err(reason.to_owned()),
            shown: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, (&'static str, ToastNotification)>> {
        self.shown.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn next_number(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// Whether a toast would be seen right now: AUMID registered, notifications enabled for it, and no « Ne pas
    /// déranger ».
    pub fn state(&self) -> ToastState {
        if let Err(reason) = &self.registered {
            return ToastState::Unavailable(format!("identité des notifications non enregistrée : {reason}"));
        }
        let setting = notifier().and_then(|n| n.Setting()).ok().map(|s| s.0);
        let mode = ToastNotificationManager::GetDefault()
            .and_then(|m| m.NotificationMode())
            .ok()
            .map(|m| m.0);
        ToastState::from_winrt(setting, mode)
    }

    /// Shows `content`, removed by Windows at `expires` if it is still there. Every event goes to `on_event`, on a
    /// thread of WinRT.
    pub fn show<F>(&self, content: &ToastContent, expires: Option<SystemTime>, on_event: F) -> Result<(), String>
    where
        F: Fn(ToastEvent) + Clone + Send + 'static,
    {
        let failed = |step: &str, error: windows::core::Error| format!("{step} (code {:#010x})", error.code().0);
        let doc = document(content).map_err(|e| failed("contenu de la notification refusé", e))?;
        let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| failed("notification refusée", e))?;
        toast
            .SetTag(&HSTRING::from(content.tag.as_str()))
            .and_then(|()| toast.SetGroup(&HSTRING::from(content.group)))
            .and_then(|()| toast.SetExpiresOnReboot(true))
            .map_err(|e| failed("notification non étiquetée", e))?;
        if let Some(ticks) = expires.and_then(windows_ticks) {
            PropertyValue::CreateDateTime(DateTime { UniversalTime: ticks })
                .and_then(|value| value.cast::<IReference<DateTime>>())
                .and_then(|value| toast.SetExpirationTime(&value))
                .map_err(|e| failed("échéance de la notification refusée", e))?;
        }
        let activated = on_event.clone();
        toast
            .Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
                move |_, args| {
                    activated(ToastEvent::Activated(activated_arguments(&args)));
                    Ok(())
                },
            ))
            .map_err(|e| failed("clic de la notification non suivi", e))?;
        let dismissed = on_event.clone();
        toast
            .Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(
                move |_, args| {
                    dismissed(ToastEvent::Dismissed(dismissal(&args)));
                    Ok(())
                },
            ))
            .map_err(|e| failed("fermeture de la notification non suivie", e))?;
        toast
            .Failed(&TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(
                move |_, args| {
                    let code = args.as_ref().and_then(|a| a.ErrorCode().ok()).map_or(0, |c| c.0);
                    tracing::warn!(code, "notification non affichée par Windows");
                    on_event(ToastEvent::Failed);
                    Ok(())
                },
            ))
            .map_err(|e| failed("échec de la notification non suivi", e))?;
        notifier()
            .and_then(|n| n.Show(&toast))
            .map_err(|e| failed("notification non affichée", e))?;
        self.lock().insert(content.tag.clone(), (content.group, toast));
        Ok(())
    }

    /// Removes the toast `tag` from the screen and from the notification centre, at best.
    pub fn hide(&self, tag: &str) {
        let Some((group, toast)) = self.lock().remove(tag) else {
            return;
        };
        if let Err(error) = notifier().and_then(|n| n.Hide(&toast)) {
            tracing::info!(code = error.code().0, "notification non retirée de l'écran");
        }
        let removed = ToastNotificationManager::History()
            .and_then(|h| h.RemoveGroupedTagWithId(&HSTRING::from(tag), &HSTRING::from(group), &HSTRING::from(AUMID)));
        if let Err(error) = removed {
            tracing::info!(
                code = error.code().0,
                "notification non retirée du centre de notifications"
            );
        }
    }

    /// Every toast of the application, at exit.
    pub fn clear_all(&self) {
        self.lock().clear();
        let _ = ToastNotificationManager::History().and_then(|h| h.ClearWithId(&HSTRING::from(AUMID)));
    }
}
