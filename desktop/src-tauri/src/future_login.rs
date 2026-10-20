//! Entry-owned browser presentation for shared OAuth/account services.
pub(crate) use future_host::future_login::*;

pub async fn start() -> Result<FutureLoginStart, crate::AppError> {
    start_with_browser(true).await
}

pub(crate) async fn start_with_browser(open: bool) -> Result<FutureLoginStart, crate::AppError> {
    let login = future_host::future_login::start().await?;
    if open {
        open_browser(&login.verification_uri_complete);
    }
    Ok(login)
}

/// Open a URL in the default browser, detached and best-effort (mirrors the
/// CLI). The shared authorization service validates the URL before returning it.
/// Uses the `open` crate (ShellExecuteW on Windows) — never `cmd /c start`,
/// which re-parses the URL so a `&` truncates it and `&cmd`-style payloads
/// from a hostile platform host would execute.
fn open_browser(url: &str) {
    // The opener is swappable so tests can assert the URL is passed through
    // without launching a real browser (`open::that_detached` is a real side
    // effect that would escape the test process).
    let opener = BROWSER_OPENER.get_or_init(|| |url: &str| open::that_detached(url));
    let _ = opener(url);
}

/// Swappable default-browser opener; production uses the default `open` crate
/// launcher, tests install a no-op.
static BROWSER_OPENER: std::sync::OnceLock<fn(&str) -> std::io::Result<()>> =
    std::sync::OnceLock::new();

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn open_browser_uses_injected_opener() {
        let _ = BROWSER_OPENER.set(|_| Ok(()));
        open_browser("https://example.com/device");
    }
}
