//! Update notification: check GitHub's "latest release" API and compare it with
//! the running version, so the app can point the user at a newer build.
//!
//! Notify-only by design (see the plan): a shipped release is an ad-hoc-signed
//! `.app` inside a `.dmg`, which cannot be swapped in place without breaking its
//! signature, so this never touches the binary. It only surfaces a banner /
//! dialog linking to the release page.
//!
//! The pure functions here (version parsing, comparison, throttling, URL and
//! JSON handling) are unit-tested with fixture bytes and no network. The thread
//! wrapper at the bottom performs the one blocking HTTP GET off the UI thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

/// Minimum interval between automatic startup checks (24 hours, in seconds).
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// How long the HTTP request may run before it is abandoned.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Sent as the `User-Agent`; GitHub rejects API requests that omit it (403).
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// The subset of GitHub's release JSON we read. Container-level `#[serde(default)]`
/// fills any missing field from `Default` (empty strings) instead of failing.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Release {
    tag_name: String,
    html_url: String,
}

/// The result of one update check, from the app's point of view.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OutcomeKind {
    /// The running version is the latest; carries it for the dialog.
    UpToDate(String),
    /// A newer release exists: its display version and the page to open.
    Available { version: String, url: String },
    /// The check could not complete; carries a short reason for the dialog.
    Failed(String),
}

/// One completed check plus whether the user asked for it. Manual checks show a
/// dialog for every outcome; automatic ones are silent unless an update exists.
#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) manual: bool,
    pub(crate) kind: OutcomeKind,
}

/// Parse a `vMAJOR.MINOR.PATCH` (or bare `MAJOR.MINOR.PATCH`) version into a
/// comparable triple. A pre-release / build suffix (`-rc1`, `+meta`) is dropped
/// before parsing, and anything malformed yields `None` — so a tag we cannot
/// understand is never treated as "newer".
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);
    let core = s.split(['-', '+']).next().unwrap_or(s);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    // Reject extra dotted components ("1.2.3.4") rather than silently accepting.
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether `latest_tag` names a strictly newer release than `current`. Either
/// side failing to parse yields `false` (never a false "update available").
fn is_newer(current: &str, latest_tag: &str) -> bool {
    match (parse_version(current), parse_version(latest_tag)) {
        (Some(cur), Some(latest)) => latest > cur,
        _ => false,
    }
}

/// Whether an automatic check is due: never run before, or last run at least
/// [`CHECK_INTERVAL_SECS`] ago. `saturating_sub` turns a future timestamp (clock
/// skew) into simply "not due yet".
pub(crate) fn should_check(now: u64, last: Option<u64>) -> bool {
    match last {
        None => true,
        Some(last) => now.saturating_sub(last) >= CHECK_INTERVAL_SECS,
    }
}

/// Extract the `owner/repo` slug from a GitHub repository URL, or `None` if it
/// is not a recognizable GitHub URL.
fn repo_slug(repo_url: &str) -> Option<String> {
    let rest = repo_url.trim_end_matches('/').split("github.com/").nth(1)?;
    let mut parts = rest.split('/');
    let owner = parts.next().filter(|s| !s.is_empty())?;
    let repo = parts.next().filter(|s| !s.is_empty())?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    Some(format!("{owner}/{repo}"))
}

/// GitHub REST endpoint for a repo's newest published (non-draft, non-pre)
/// release.
fn api_url(slug: &str) -> String {
    format!("https://api.github.com/repos/{slug}/releases/latest")
}

/// Human-facing "latest release" page, used when the API omits `html_url`.
fn release_page_url(slug: &str) -> String {
    format!("https://github.com/{slug}/releases/latest")
}

/// The version string shown to the user: the tag with a leading `v` removed
/// (`v0.5.0` -> `0.5.0`), so the banner reads naturally.
fn display_version(tag: &str) -> String {
    let tag = tag.trim();
    tag.strip_prefix('v').unwrap_or(tag).to_string()
}

/// Parse GitHub's release JSON, keeping only the fields we use.
fn parse_release(body: &[u8]) -> Option<Release> {
    serde_json::from_slice(body).ok()
}

/// Turn a release-API response into an outcome for the given running version.
fn evaluate(body: &[u8], current: &str, slug: &str) -> OutcomeKind {
    let Some(release) = parse_release(body) else {
        return OutcomeKind::Failed("could not read release data".to_string());
    };
    if is_newer(current, &release.tag_name) {
        // Only open an https URL from the payload; anything else (missing field,
        // unexpected scheme) falls back to the repo's own releases page.
        let url = if release.html_url.starts_with("https://") {
            release.html_url
        } else {
            release_page_url(slug)
        };
        OutcomeKind::Available {
            version: display_version(&release.tag_name),
            url,
        }
    } else {
        OutcomeKind::UpToDate(display_version(current))
    }
}

/// The most recent completed check, awaiting the UI thread to drain it.
static RESULT: Mutex<Option<Outcome>> = Mutex::new(None);

/// True while a check is running, so overlapping requests are ignored.
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Current wall-clock time in whole seconds since the Unix epoch (0 if the
/// system clock is somehow before it).
pub(crate) fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Take the latest completed check result, if any (leaving none behind).
pub(crate) fn take_result() -> Option<Outcome> {
    RESULT.lock().unwrap().take()
}

/// Kick off a background update check. Does nothing if one is already running.
/// When it finishes it stores an [`Outcome`] for [`take_result`] and wakes the
/// UI with `ctx.request_repaint()`.
pub(crate) fn spawn_check(ctx: egui::Context, manual: bool) {
    // Ignore a second request while one is in flight (e.g. repeated menu clicks).
    if IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        // Reset the guard even if the check panics, so a check stays possible
        // for the rest of the session.
        let _reset = InFlightReset;
        let kind = run_check();
        *RESULT.lock().unwrap() = Some(Outcome { manual, kind });
        ctx.request_repaint();
    });
}

/// Clears [`IN_FLIGHT`] on drop (including during a panic unwind).
struct InFlightReset;

impl Drop for InFlightReset {
    fn drop(&mut self) {
        IN_FLIGHT.store(false, Ordering::SeqCst);
    }
}

/// The blocking body of a check: resolve the repo, GET the release JSON, and
/// evaluate it. Every failure path becomes an [`OutcomeKind::Failed`].
fn run_check() -> OutcomeKind {
    let Some(slug) = repo_slug(env!("CARGO_PKG_REPOSITORY")) else {
        return OutcomeKind::Failed("no repository configured".to_string());
    };
    match fetch(&api_url(&slug)) {
        Ok(body) => evaluate(&body, env!("CARGO_PKG_VERSION"), &slug),
        Err(e) => OutcomeKind::Failed(e),
    }
}

/// Perform the HTTP GET, returning the response body bytes or a short error.
fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .new_agent();
    agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_vec()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_handles_prefix_and_suffixes() {
        assert_eq!(parse_version("v0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("0.4.0"), Some((0, 4, 0)));
        assert_eq!(parse_version("v1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3+build.5"), Some((1, 2, 3)));
        assert_eq!(parse_version("  v2.0.10 "), Some((2, 0, 10)));
    }

    #[test]
    fn parse_version_rejects_malformed() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("v"), None);
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("x.y.z"), None);
        assert_eq!(parse_version("latest"), None);
    }

    #[test]
    fn is_newer_only_on_strictly_greater() {
        assert!(is_newer("0.4.0", "v0.5.0"));
        assert!(is_newer("0.4.0", "v0.4.1"));
        assert!(is_newer("0.4.0", "v1.0.0"));
        assert!(!is_newer("0.4.0", "v0.4.0"));
        assert!(!is_newer("0.4.0", "v0.3.9"));
        // A version we cannot parse is never "newer".
        assert!(!is_newer("0.4.0", "nightly"));
        assert!(!is_newer("garbage", "v9.9.9"));
    }

    #[test]
    fn should_check_respects_the_interval() {
        let day = CHECK_INTERVAL_SECS;
        assert!(should_check(day, None)); // never checked
        assert!(should_check(2 * day, Some(day - 1))); // > 24h ago
        assert!(should_check(2 * day, Some(day))); // exactly 24h ago
        assert!(!should_check(day, Some(day - 1))); // < 24h ago
                                                    // Clock skew: a "future" last-check is simply not due yet.
        assert!(!should_check(day, Some(2 * day)));
    }

    #[test]
    fn repo_slug_extracts_owner_and_repo() {
        let want = Some("sndpl/msx-media-explorer");
        assert_eq!(
            repo_slug("https://github.com/sndpl/msx-media-explorer").as_deref(),
            want
        );
        assert_eq!(
            repo_slug("https://github.com/sndpl/msx-media-explorer/").as_deref(),
            want
        );
        assert_eq!(
            repo_slug("https://github.com/sndpl/msx-media-explorer.git").as_deref(),
            want
        );
        assert_eq!(repo_slug("https://example.com/foo/bar"), None);
        assert_eq!(repo_slug("not a url"), None);
    }

    #[test]
    fn url_builders_use_the_slug() {
        assert_eq!(
            api_url("a/b"),
            "https://api.github.com/repos/a/b/releases/latest"
        );
        assert_eq!(
            release_page_url("a/b"),
            "https://github.com/a/b/releases/latest"
        );
    }

    #[test]
    fn display_version_strips_v_prefix() {
        assert_eq!(display_version("v0.5.0"), "0.5.0");
        assert_eq!(display_version("0.5.0"), "0.5.0");
    }

    /// A minimal GitHub "latest release" payload for the parse/evaluate tests.
    fn release_json(tag: &str, url: &str) -> Vec<u8> {
        format!(r#"{{"tag_name":"{tag}","html_url":"{url}","name":"ignored"}}"#).into_bytes()
    }

    #[test]
    fn parse_release_reads_tag_and_url() {
        let r = parse_release(&release_json("v0.5.0", "https://example/rel")).unwrap();
        assert_eq!(r.tag_name, "v0.5.0");
        assert_eq!(r.html_url, "https://example/rel");
    }

    #[test]
    fn parse_release_tolerates_missing_fields_and_rejects_garbage() {
        let r = parse_release(br#"{"name":"only"}"#).unwrap();
        assert_eq!(r.tag_name, "");
        assert_eq!(r.html_url, "");
        assert!(parse_release(b"not json").is_none());
    }

    #[test]
    fn evaluate_reports_available_with_release_url() {
        let url = "https://github.com/sndpl/msx-media-explorer/releases/tag/v0.5.0";
        let kind = evaluate(
            &release_json("v0.5.0", url),
            "0.4.0",
            "sndpl/msx-media-explorer",
        );
        assert_eq!(
            kind,
            OutcomeKind::Available {
                version: "0.5.0".to_string(),
                url: url.to_string(),
            }
        );
    }

    #[test]
    fn evaluate_falls_back_to_release_page_when_url_missing_or_not_https() {
        let fallback = "https://github.com/sndpl/msx-media-explorer/releases/latest";
        for html_url in ["", "http://insecure.example/rel", "file:///etc/passwd"] {
            let kind = evaluate(
                &release_json("v0.5.0", html_url),
                "0.4.0",
                "sndpl/msx-media-explorer",
            );
            assert_eq!(
                kind,
                OutcomeKind::Available {
                    version: "0.5.0".to_string(),
                    url: fallback.to_string(),
                }
            );
        }
    }

    #[test]
    fn evaluate_reports_up_to_date_on_same_or_older() {
        let same = release_json("v0.4.0", "https://x");
        assert_eq!(
            evaluate(&same, "0.4.0", "a/b"),
            OutcomeKind::UpToDate("0.4.0".to_string())
        );
        let older = release_json("v0.3.0", "https://x");
        assert_eq!(
            evaluate(&older, "0.4.0", "a/b"),
            OutcomeKind::UpToDate("0.4.0".to_string())
        );
    }

    #[test]
    fn evaluate_fails_on_unparseable_body() {
        assert!(matches!(
            evaluate(b"garbage", "0.4.0", "a/b"),
            OutcomeKind::Failed(_)
        ));
    }
}
