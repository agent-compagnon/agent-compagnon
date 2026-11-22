//! Hosts of the login windows: top-level navigations, remote URLs of the deposit capability, cookies of a session.

use url::Url;

/// `http` is only accepted towards these hosts, as for the URLs of the configuration (I1.14).
fn is_loopback(host: &str) -> bool {
    host == "localhost" || host == "127.0.0.1"
}

/// Whether a top-level navigation to `url` is allowed: host listed exactly (no subdomain, no neighbour), `https` on
/// any port, `http` only towards `localhost` and `127.0.0.1`, no identifiers in the URL. Any other scheme
/// (`about:`, `data:`, `file:`, `javascript:`…) is refused.
pub fn navigation_allowed(allowed: &[String], url: &Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let scheme_ok = match url.scheme() {
        "https" => true,
        "http" => is_loopback(host),
        _ => false,
    };
    scheme_ok && allowed.iter().any(|h| h == host)
}

/// What a log line may say of a destination: scheme, host and port, never the path, the query or the fragment
/// (they may carry anything, a token included).
pub fn destination_for_log(url: &Url) -> String {
    match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{}://{host}:{port}", url.scheme()),
        (Some(host), None) => format!("{}://{host}", url.scheme()),
        (None, _) => format!("{}:", url.scheme()),
    }
}

/// A plain host name or IPv4 address: labels of `[a-z0-9-]`, neither empty nor starting or ending with `-`. Nothing
/// else can be written into a remote URL pattern, whose syntax gives meaning to `*`, `:`, `(`, `{`…
fn is_plain_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// The remote URLs of the deposit capability of a window, one or two per host of `allowed_navigation`:
/// `https://{host}:*`, plus `http://{host}:*` for `localhost` and `127.0.0.1`. Patterns are matched against the
/// origin of the calling page (path, query and fragment are wildcards in Tauri). `None` if a host is not a plain
/// host name: no capability is then created and the window is not opened.
pub fn remote_url_patterns(allowed: &[String]) -> Option<Vec<String>> {
    if allowed.is_empty() {
        return None;
    }
    let mut patterns = Vec::with_capacity(allowed.len());
    for host in allowed {
        if !is_plain_host(host) {
            return None;
        }
        patterns.push(format!("https://{host}:*"));
        if is_loopback(host) {
            patterns.push(format!("http://{host}:*"));
        }
    }
    Some(patterns)
}

/// Whether a cookie of `domain` would be sent to one of `hosts` (RFC 6265 domain match, leading dot ignored): the
/// cookies that « Déconnecter » removes for a session.
pub fn cookie_domain_matches(domain: &str, hosts: &[String]) -> bool {
    let domain = domain.trim_start_matches('.').to_ascii_lowercase();
    if domain.is_empty() {
        return false;
    }
    let suffix = format!(".{domain}");
    hosts.iter().any(|host| *host == domain || host.ends_with(&suffix))
}

/// Whether `url` is a page of the application bundle, as Tauri serves it on Windows (`http://tauri.localhost/…`,
/// `https://` with `use_https_scheme`): the only pages of the Basic window, which has no remote content.
pub fn app_page_allowed(url: &Url) -> bool {
    url.username().is_empty()
        && url.password().is_none()
        && matches!(url.scheme(), "http" | "https")
        && url.host_str() == Some("tauri.localhost")
        && url.port().is_none()
}
