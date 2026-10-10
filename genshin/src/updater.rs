//! Auto-update: check GitHub for new releases and self-replace the executable.
//!
//! Version scheme: CalVer tags (e.g. `v2026.03.27`) mapped to semver
//! `YYYYMMDD.build.0` in Cargo.toml by CI. Dev builds (major < 20000000)
//! skip the update check entirely.
//!
//! **Version check** uses two strategies:
//!   1. GitHub REST API (`api.github.com`) — fast, structured JSON
//!   2. Redirect fallback (`/releases/latest` → 302 Location header) —
//!      works through download mirrors when the API is blocked
//!
//! **Downloads** use the same mirror chain as the ONNX Runtime download:
//!   gh-proxy.com → ghfast.top → direct GitHub

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use yas::{log_debug, log_info, log_warn};

/// Asset filename for the OCR scanner binary.
pub const ASSET_SCANNER: &str = "GGScannerOCR.exe";
/// Asset filename for the merged packet capture + OCR/manager binary.
pub const ASSET_CAPTURE: &str = "GGScanner.exe";
/// Keep the original URL during the bridge; GitHub redirects it after renaming.
pub const RELEASES_URL: &str = "https://github.com/Anyrainel/GOODScanner/releases/latest";

/// Download mirror prefixes, tried in order.  Empty string = direct GitHub.
const DOWNLOAD_MIRRORS: &[&str] = &[
    "https://gh-proxy.com/",
    "https://ghfast.top/",
    "", // direct GitHub
];

/// Try the future repository first, while supporting the transition release
/// before the repository is renamed. Old installed updaters still need release
/// assets published under GOODScanner/GOODCapture aliases after the rename.
const RELEASE_REPOSITORIES: &[&str] = &["Anyrainel/GGScanner", "Anyrainel/GOODScanner"];

/// Minimum plausible exe size (1 MB).  The real binary is 20+ MB;
/// anything smaller is almost certainly an error page or truncated download.
const MIN_EXE_SIZE: usize = 1_000_000;

// ── GitHub API types (minimal) ───────────────────────────────────

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    // None distinguishes an unavailable inventory from a known empty release.
    assets: Option<Vec<GitHubAsset>>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
}

#[derive(Deserialize)]
struct ReleaseBuild {
    tag: String,
    revision: u32,
    /// Additive to the existing manifest: old updaters ignore these fields.
    /// Stable roles let a transition build find future renamed binaries.
    #[serde(default)]
    assets: ReleaseAssets,
}

#[derive(Default, Deserialize)]
struct ReleaseAssets {
    scanner: Option<String>,
    capture: Option<String>,
}

struct ResolvedRelease {
    repository: &'static str,
    tag: String,
    revision: u32,
    assets: ReleaseAssets,
    published_assets: Option<Vec<GitHubAsset>>,
}

impl ResolvedRelease {
    fn download_url(&self, asset_name: &str) -> Result<Option<String>> {
        let (mapped, legacy, renamed) = match asset_name {
            ASSET_SCANNER | "GOODScanner.exe" => (
                self.assets.scanner.as_deref(),
                "GOODScanner.exe",
                ASSET_SCANNER,
            ),
            ASSET_CAPTURE | "GOODCapture.exe" => (
                self.assets.capture.as_deref(),
                "GOODCapture.exe",
                ASSET_CAPTURE,
            ),
            _ => return Err(anyhow!("Unknown updater edition: {asset_name}")),
        };
        let mut candidates = Vec::new();
        if let Some(filename) = mapped {
            if !filename.ends_with(".exe")
                || !filename
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            {
                return Err(anyhow!("Invalid release executable filename: {filename}"));
            }
            candidates.push(filename.to_owned());
        }
        // Without a manifest, prefer the still-published old names. The API
        // inventory also allows a renamed client to update from legacy releases.
        candidates.push(revision_asset_name(legacy, self.revision));
        candidates.push(revision_asset_name(renamed, self.revision));
        candidates.push(legacy.to_owned());
        candidates.push(renamed.to_owned());
        let filename = match &self.published_assets {
            Some(assets) => candidates
                .iter()
                .find(|name| assets.iter().any(|asset| &asset.name == *name)),
            None => candidates.first(),
        };
        let Some(filename) = filename else {
            return Ok(None);
        };
        Ok(Some(format!(
            "https://github.com/{}/releases/download/{}/{}",
            self.repository, self.tag, filename
        )))
    }
}

fn release_revision(body: Option<&str>) -> u32 {
    body.and_then(|body| {
        body.lines()
            .find_map(|line| line.strip_prefix("Build revision: ")?.trim().parse().ok())
    })
    .unwrap_or(0)
}

fn current_revision() -> u32 {
    env!("CARGO_PKG_VERSION")
        .split('.')
        .nth(1)
        .and_then(|minor| minor.parse().ok())
        .unwrap_or(0)
}

// ── Public types ─────────────────────────────────────────────────

/// Result of an update check.
pub enum UpdateStatus {
    /// Already on the latest (or newer) version.
    UpToDate,
    /// A newer release exists on GitHub.
    UpdateAvailable {
        current_version: String,
        latest_version: String,
        /// Direct github.com download URL for the exe asset.
        download_url: String,
    },
    /// A newer release exists, but has no executable for the installed edition.
    ManualDownload { latest_version: String },
    /// Running a dev build — skip update checks.
    DevBuild,
}

// ── Version helpers ──────────────────────────────────────────────

/// Parse a CalVer tag like `"v2026.03.27"` → `20260327u32`.
fn parse_calver_tag(tag: &str) -> Option<u32> {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<&str> = tag.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: u32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;
    if year < 2020 || month < 1 || month > 12 || day < 1 || day > 31 {
        return None;
    }
    Some(year * 10000 + month * 100 + day)
}

/// The current build's CalVer integer, or `None` for dev builds.
///
/// CI sets `CARGO_PKG_VERSION` to `YYYYMMDD.build.0`; the major component
/// is ≥ 20000000.  Local dev builds have `0.x.y` (major < 20000000).
fn current_version_int() -> Option<u32> {
    let version = env!("CARGO_PKG_VERSION");
    let major: u32 = version.split('.').next()?.parse().ok()?;
    if major < 20000000 {
        return None; // dev build
    }
    Some(major)
}

/// Window title including version, e.g. `"GOOD Scanner v2026.05.17"`.
pub fn window_title(product: &str) -> String {
    format!("{} {}", product, current_version_display())
}

/// Human-readable current version string.
pub fn current_version_display() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let major: u32 = version
        .split('.')
        .next()
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    if major >= 20000000 {
        let year = major / 10000;
        let month = (major % 10000) / 100;
        let day = major % 100;
        let revision = current_revision();
        if revision == 0 {
            format!("v{}.{:02}.{:02}", year, month, day)
        } else {
            format!("v{}.{:02}.{:02} (build {})", year, month, day, revision)
        }
    } else {
        format!("v{}", version)
    }
}

// ── Tag resolution strategies ────────────────────────────────────

/// Strategy 1: GitHub REST API (fast, works in most regions).
fn get_tag_via_api() -> Option<ResolvedRelease> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .user_agent("GOODScanner-Updater")
        .build()
        .ok()?;

    for &repository in RELEASE_REPOSITORIES {
        let url = format!("https://api.github.com/repos/{repository}/releases/latest");
        log_debug!("检查更新(API): {}", "Checking via API: {}", url);
        let Ok(resp) = client.get(url).send() else {
            continue;
        };
        if !resp.status().is_success() {
            log_debug!("API 返回: {}", "API returned: {}", resp.status());
            continue;
        }
        let Ok(release) = resp.json::<GitHubRelease>() else {
            continue;
        };
        if parse_calver_tag(&release.tag_name).is_some() {
            return Some(ResolvedRelease {
                repository,
                tag: release.tag_name,
                revision: release_revision(release.body.as_deref()),
                assets: ReleaseAssets::default(),
                published_assets: release.assets,
            });
        }
    }
    None
}

/// The daily tag stays compatible with old updaters. A small mirrored asset
/// carries the revision when the GitHub API is unavailable.
fn get_build_via_mirrors(repository: &str, tag: &str) -> Option<ReleaseBuild> {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .user_agent("GOODScanner-Updater")
        .build()
    else {
        return None;
    };
    let check = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    for mirror in DOWNLOAD_MIRRORS {
        let url = format!(
            "{mirror}https://github.com/{repository}/releases/download/{tag}/update.json?check={check}"
        );
        if let Ok(response) = client.get(url).send() {
            if response.status().is_success() {
                if let Ok(build) = response.json::<ReleaseBuild>() {
                    if build.tag == tag {
                        return Some(build);
                    }
                }
            }
        }
    }
    None // Legacy releases have no revision asset.
}

/// Strategy 2: Follow `/releases/latest` redirect through download mirrors.
///
/// A repository rename can add redirects before `/releases/tag/vYYYY.MM.DD`.
/// Follow those hops manually and stop at the tag, without downloading HTML.
/// Tried through each mirror prefix so it works when github.com is blocked.
fn get_tag_via_redirect() -> Option<ResolvedRelease> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("GOODScanner-Updater")
        .build()
        .ok()?;

    for &repository in RELEASE_REPOSITORIES {
        for mirror in DOWNLOAD_MIRRORS {
            let url = format!("{mirror}https://github.com/{repository}/releases/latest");
            log_debug!("检查更新(redirect): {}", "Checking via redirect: {}", url);
            if let Some(tag) = follow_release_redirects(&client, &url) {
                return Some(ResolvedRelease {
                    repository,
                    tag,
                    revision: 0,
                    assets: ReleaseAssets::default(),
                    published_assets: None,
                });
            }
        }
    }
    None
}

fn follow_release_redirects(client: &reqwest::blocking::Client, url: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(url).ok()?;
    for _ in 0..5 {
        let resp = match client.get(url.clone()).send() {
            Ok(r) => r,
            Err(e) => {
                log_debug!("连接失败: {}", "Connection failed: {}", e);
                return None;
            },
        };

        // Look for 3xx redirect with Location header
        if !resp.status().is_redirection() {
            log_debug!(
                "非重定向响应: {}",
                "Non-redirect response: {}",
                resp.status()
            );
            return None;
        }

        let location = match resp.headers().get("location") {
            Some(v) => match v.to_str() {
                Ok(s) => s.to_string(),
                Err(_) => return None,
            },
            None => return None,
        };

        if let Some(tag) = extract_tag_from_url(&location) {
            return Some(tag.to_string());
        }
        url = url.join(&location).ok()?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
    }

    None
}

/// Extract a CalVer tag from a URL containing `/releases/tag/vX.Y.Z`.
fn extract_tag_from_url(url: &str) -> Option<&str> {
    let marker = "/releases/tag/";
    let rest = url.split(marker).nth(1)?;
    // Tag ends at next '/', '?', '#', or end-of-string
    let tag = rest.split(&['/', '?', '#'][..]).next()?;
    // Validate it parses as CalVer
    if parse_calver_tag(tag).is_some() {
        Some(tag)
    } else {
        None
    }
}

// ── Public: update check ─────────────────────────────────────────

/// Query GitHub for the latest release and compare with the running version.
///
/// Tries the REST API first (structured, fast), then falls back to the
/// redirect-based approach through download mirrors (works in China when
/// `api.github.com` is blocked).
/// Query GitHub for the latest release and compare with the running version.
///
/// `asset_name` selects which binary to check for (e.g. [`ASSET_SCANNER`] or
/// [`ASSET_CAPTURE`]).  The download URL in [`UpdateStatus::UpdateAvailable`]
/// points to that specific asset.
pub fn check_for_update(asset_name: &str) -> Result<UpdateStatus> {
    let current_int = match current_version_int() {
        Some(v) => v,
        None => return Ok(UpdateStatus::DevBuild),
    };

    // Try API first, then redirect fallback
    let mut release = get_tag_via_api()
        .or_else(|| {
            log_debug!("API失败，尝试redirect方式", "API failed, trying redirect");
            get_tag_via_redirect()
        })
        .ok_or_else(|| anyhow!("无法获取最新版本信息 / Cannot determine latest version"))?;

    // Read the role map even when the API works: it decouples installed
    // transition binaries from the names chosen for subsequent releases.
    if let Some(build) = get_build_via_mirrors(release.repository, &release.tag) {
        // A cached manifest from an earlier same-day build must not override
        // the revision/filenames announced by the API.
        if build.revision >= release.revision {
            if build.revision > release.revision {
                // The manifest can arrive before a refreshed API response.
                // That older response cannot establish which assets exist now.
                release.published_assets = None;
            }
            release.revision = build.revision;
            release.assets = build.assets;
        }
    }
    resolve_update(release, asset_name, current_int, current_revision())
}

fn resolve_update(
    release: ResolvedRelease,
    asset_name: &str,
    current_int: u32,
    current_revision: u32,
) -> Result<UpdateStatus> {
    let latest_int = parse_calver_tag(&release.tag)
        .ok_or_else(|| anyhow!("无法解析版本号 / Cannot parse release tag: {}", release.tag))?;

    if (latest_int, release.revision) <= (current_int, current_revision) {
        return Ok(UpdateStatus::UpToDate);
    }

    let download_url = release.download_url(asset_name)?;
    let latest_version = if release.revision == 0 {
        release.tag
    } else {
        format!("{} (build {})", release.tag, release.revision)
    };
    let Some(download_url) = download_url else {
        return Ok(UpdateStatus::ManualDownload { latest_version });
    };
    Ok(UpdateStatus::UpdateAvailable {
        current_version: current_version_display(),
        latest_version,
        download_url,
    })
}

fn revision_asset_name(asset_name: &str, revision: u32) -> String {
    match asset_name.strip_suffix(".exe") {
        Some(stem) if revision > 0 => format!("{stem}-{revision}.exe"),
        _ => asset_name.to_string(),
    }
}

// ── Public: cleanup ──────────────────────────────────────────────

/// Delete the leftover `.old` executable from a previous update.
/// Safe to call unconditionally at every startup.
///
/// On Windows the old process may still be exiting when the new one starts
/// (after the "restart now?" dialog), so the file may be locked briefly.
/// We retry a few times with a short delay before giving up.
pub fn cleanup_old_exe() {
    if let Ok(exe) = std::env::current_exe() {
        let old = exe.with_extension("exe.old");
        if !old.exists() {
            return;
        }
        for attempt in 0..5 {
            match std::fs::remove_file(&old) {
                Ok(()) => {
                    log_debug!("已清理旧版本", "Cleaned up old exe");
                    return;
                },
                Err(e) => {
                    if attempt < 4 {
                        std::thread::sleep(Duration::from_millis(500));
                    } else {
                        log_warn!(
                            "无法清理上次更新留下的旧程序文件；当前版本仍可继续使用。完整错误详情: {}",
                            "The old application file left by the previous update could not be removed; the current version can still be used. Full error details: {}",
                            e
                        );
                    }
                },
            }
        }
    }
}

// ── Public: download & self-replace ──────────────────────────────

/// Download the new release and replace the running executable.
///
/// On Windows the running exe cannot be overwritten, but it *can* be
/// renamed.  The sequence is:
///
/// 1. Write and flush the complete download to a staging file
/// 2. Rename the current executable → `<current>.exe.old`
/// 3. Rename the staging file to the current executable path
/// 4. On next launch, `cleanup_old_exe()` removes the `.old` file
///
/// If writing the new file fails, the rename is rolled back so the
/// original exe is restored.
pub fn download_and_replace(download_url: &str) -> Result<PathBuf> {
    let exe_path = std::env::current_exe()
        .map_err(|e| anyhow!("无法获取当前程序路径 / Cannot get current exe path: {}", e))?;
    let old_path = exe_path.with_extension("exe.old");

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(15))
        .user_agent("GOODScanner-Updater")
        .build()?;

    let mut last_error = String::new();

    for (i, mirror) in DOWNLOAD_MIRRORS.iter().enumerate() {
        let url = if mirror.is_empty() {
            download_url.to_string()
        } else {
            format!("{}{}", mirror, download_url)
        };

        log_info!(
            "尝试下载源 {}/{}: {}",
            "Trying source {}/{}: {}",
            i + 1,
            DOWNLOAD_MIRRORS.len(),
            url,
        );

        match client.get(&url).send() {
            Ok(resp) if resp.status().is_success() => match resp.bytes() {
                Ok(bytes) => {
                    log_info!("下载完成（{} 字节）", "Downloaded ({} bytes)", bytes.len(),);

                    // Must be a PE executable of plausible size
                    if bytes.get(..2) != Some(b"MZ") {
                        last_error = "下载文件不是有效的exe / Not a valid PE executable".into();
                        log_warn!("{}", "{}", yas::lang::localize(&last_error));
                        continue;
                    }
                    if bytes.len() < MIN_EXE_SIZE {
                        last_error = format!(
                            "文件过小（{} 字节 < {} 字节）/ File too small ({} < {} bytes)",
                            bytes.len(),
                            MIN_EXE_SIZE,
                            bytes.len(),
                            MIN_EXE_SIZE,
                        );
                        log_warn!("{}", "{}", yas::lang::localize(&last_error));
                        continue;
                    }

                    replace_executable(&exe_path, &old_path, &bytes)?;

                    log_info!("更新完成！请重启程序。", "Update complete! Please restart.");
                    return Ok(exe_path);
                },
                Err(e) => {
                    last_error = format!("{}", e);
                    log_warn!("下载失败: {}", "Download failed: {}", last_error);
                },
            },
            Ok(resp) => {
                last_error = format!("HTTP {}", resp.status());
                log_warn!("源 {} 失败: {}", "Source {} failed: {}", i + 1, last_error,);
            },
            Err(e) => {
                last_error = format!("{}", e);
                log_warn!("连接失败: {}", "Connection failed: {}", last_error);
            },
        }
    }

    Err(anyhow!(
        "所有下载源均失败 / All download sources failed: {}",
        last_error
    ))
}

fn replace_executable(exe_path: &Path, old_path: &Path, bytes: &[u8]) -> Result<()> {
    let staged = exe_path.with_extension(format!("exe.{}.new", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .with_context(|| format!("Cannot stage update: {}", staged.display()))?;
    let result = (|| {
        file.write_all(bytes)
            .context("Cannot write staged update")?;
        file.sync_all().context("Cannot flush staged update")?;
        drop(file);
        match std::fs::remove_file(old_path) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error).context(
                "Cannot remove previous update backup; close the previous application before updating again",
            ),
        }
        std::fs::rename(exe_path, old_path).context("Cannot rename current executable")?;
        if let Err(error) = std::fs::rename(&staged, exe_path) {
            std::fs::rename(old_path, exe_path).with_context(|| {
                format!("Cannot restore original executable after installation failed: {error}; backup={}", old_path.display())
            })?;
            return Err(error)
                .context("Cannot install staged update; original executable restored");
        }
        Ok(())
    })();
    if staged.exists() {
        if let Err(error) = std::fs::remove_file(&staged) {
            log_warn!(
                "无法清理更新临时文件: {}",
                "Cannot remove staged update file: {}",
                error
            );
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_and_revised_releases_are_compatible() {
        let release: GitHubRelease = serde_json::from_str(r#"{"tag_name":"v2026.10.05"}"#).unwrap();
        assert_eq!(release_revision(release.body.as_deref()), 0);
        assert_eq!(
            release_revision(Some("Build revision: 123\n\nRelease notes")),
            123
        );
        assert_eq!(release_revision(Some("Build revision: invalid")), 0);
        assert_eq!(parse_calver_tag("v2026.10.05"), Some(20261005));
        assert_eq!(revision_asset_name("GOODCapture.exe", 0), "GOODCapture.exe");
        assert_eq!(
            revision_asset_name("GOODCapture.exe", 123),
            "GOODCapture-123.exe"
        );
        assert_eq!(
            revision_asset_name("GOODScanner.exe", 123),
            "GOODScanner-123.exe"
        );
        let build: ReleaseBuild =
            serde_json::from_str(r#"{"tag":"v2026.10.05","revision":123}"#).unwrap();
        let release = ResolvedRelease {
            repository: "Anyrainel/GOODScanner",
            tag: build.tag,
            revision: build.revision,
            assets: build.assets,
            published_assets: None,
        };
        assert_eq!(
            release.download_url("GOODScanner.exe").unwrap().unwrap(),
            "https://github.com/Anyrainel/GOODScanner/releases/download/v2026.10.05/GOODScanner-123.exe"
        );
    }

    #[test]
    fn manifest_roles_preserve_editions_across_repository_and_binary_rename() {
        let build: ReleaseBuild = serde_json::from_str(
            r#"{
            "tag":"v2026.10.07", "revision":125,
            "assets":{"scanner":"GGScannerOCR-125.exe","capture":"GGScanner-125.exe"}
        }"#,
        )
        .unwrap();
        let release = ResolvedRelease {
            repository: "Anyrainel/GGScanner",
            tag: build.tag,
            revision: build.revision,
            assets: build.assets,
            published_assets: None,
        };
        assert_eq!(
            release.download_url(ASSET_SCANNER).unwrap().unwrap(),
            "https://github.com/Anyrainel/GGScanner/releases/download/v2026.10.07/GGScannerOCR-125.exe"
        );
        assert_eq!(
            release.download_url(ASSET_CAPTURE).unwrap().unwrap(),
            "https://github.com/Anyrainel/GGScanner/releases/download/v2026.10.07/GGScanner-125.exe"
        );
        assert_eq!(
            release.download_url("GOODScanner.exe").unwrap(),
            release.download_url(ASSET_SCANNER).unwrap()
        );
        assert_eq!(
            release.download_url("GOODCapture.exe").unwrap(),
            release.download_url(ASSET_CAPTURE).unwrap()
        );
    }

    fn release_with_inventory(names: &[&str]) -> ResolvedRelease {
        ResolvedRelease {
            repository: "Anyrainel/GOODScanner",
            tag: "v2026.10.07".into(),
            revision: 0,
            assets: ReleaseAssets::default(),
            published_assets: Some(
                names
                    .iter()
                    .map(|name| GitHubAsset {
                        name: (*name).into(),
                    })
                    .collect(),
            ),
        }
    }

    #[test]
    fn revision_metadata_does_not_require_numbered_downloads() {
        let build: ReleaseBuild = serde_json::from_str(
            r#"{"tag":"v2026.10.10","revision":81,"assets":{"scanner":"GOODScanner.exe","capture":"GOODCapture.exe"}}"#,
        )
        .unwrap();
        let mut release = release_with_inventory(&["GOODScanner.exe", "GOODCapture.exe"]);
        release.tag = build.tag;
        release.revision = build.revision;
        release.assets = build.assets;
        for inventory_available in [true, false] {
            if !inventory_available {
                release.published_assets = None;
            }
            for (edition, filename) in [
                (ASSET_SCANNER, "GOODScanner.exe"),
                (ASSET_CAPTURE, "GOODCapture.exe"),
            ] {
                assert!(release
                    .download_url(edition)
                    .unwrap()
                    .unwrap()
                    .ends_with(filename));
            }
        }
        release.published_assets =
            release_with_inventory(&["GOODScanner.exe", "GOODCapture.exe"]).published_assets;
        release.assets = ReleaseAssets::default();
        assert!(release
            .download_url(ASSET_CAPTURE)
            .unwrap()
            .unwrap()
            .ends_with("/GOODCapture.exe"));
    }

    #[test]
    fn renamed_clients_find_legacy_assets_without_a_manifest() {
        let release = release_with_inventory(&["GOODScanner.exe", "GOODCapture.exe"]);
        assert!(release
            .download_url(ASSET_SCANNER)
            .unwrap()
            .unwrap()
            .ends_with("/GOODScanner.exe"));
        assert!(release
            .download_url(ASSET_CAPTURE)
            .unwrap()
            .unwrap()
            .ends_with("/GOODCapture.exe"));
        // The redirect-only path also uses the legacy names before a role map exists.
        let release = ResolvedRelease {
            published_assets: None,
            ..release
        };
        assert!(release
            .download_url(ASSET_SCANNER)
            .unwrap()
            .unwrap()
            .ends_with("/GOODScanner.exe"));
    }

    #[test]
    fn legacy_clients_find_renamed_assets_without_a_manifest() {
        let release = release_with_inventory(&["GGScannerOCR.exe", "GGScanner.exe"]);
        assert!(release
            .download_url("GOODScanner.exe")
            .unwrap()
            .unwrap()
            .ends_with("/GGScannerOCR.exe"));
        assert!(release
            .download_url("GOODCapture.exe")
            .unwrap()
            .unwrap()
            .ends_with("/GGScanner.exe"));
    }

    #[test]
    fn manifest_maps_new_client_to_bridge_assets() {
        let mut release = release_with_inventory(&["GOODScanner-125.exe", "GOODCapture-125.exe"]);
        release.revision = 125;
        release.assets = ReleaseAssets {
            scanner: Some("GOODScanner-125.exe".into()),
            capture: Some("GOODCapture-125.exe".into()),
        };
        assert!(release
            .download_url(ASSET_SCANNER)
            .unwrap()
            .unwrap()
            .ends_with("/GOODScanner-125.exe"));
        assert!(release
            .download_url(ASSET_CAPTURE)
            .unwrap()
            .unwrap()
            .ends_with("/GOODCapture-125.exe"));
    }

    #[test]
    fn missing_executable_offers_manual_download_without_changing_editions() {
        // A capture-only upload must never be offered to an OCR-only installation.
        for names in [
            &[][..],
            &["GGScanner.exe"][..],
            &["GOODCapture.exe", "update.json"][..],
        ] {
            let release = release_with_inventory(names);
            assert!(matches!(
                resolve_update(release, ASSET_SCANNER, 20261005, 0).unwrap(),
                UpdateStatus::ManualDownload { latest_version } if latest_version == "v2026.10.07"
            ));
        }
        let release = release_with_inventory(&["GGScannerOCR.exe"]);
        assert!(matches!(
            resolve_update(release, ASSET_CAPTURE, 20261005, 0).unwrap(),
            UpdateStatus::ManualDownload { .. }
        ));
        let release = release_with_inventory(&[]);
        assert!(matches!(
            resolve_update(release, ASSET_SCANNER, 20261007, 0).unwrap(),
            UpdateStatus::UpToDate
        ));
    }

    #[test]
    fn unavailable_manifest_asset_uses_matching_alias_or_manual_download() {
        let mut release = release_with_inventory(&["GOODScanner-125.exe"]);
        release.revision = 125;
        release.assets.scanner = Some("GGScannerOCR-125.exe".into());
        assert!(release
            .download_url(ASSET_SCANNER)
            .unwrap()
            .unwrap()
            .ends_with("/GOODScanner-125.exe"));
        release.published_assets = Some(Vec::new());
        assert!(matches!(
            resolve_update(release, ASSET_SCANNER, 20261005, 0).unwrap(),
            UpdateStatus::ManualDownload { .. }
        ));
    }

    #[test]
    fn manifest_asset_must_be_an_executable_filename() {
        for invalid in [
            "../GGScanner.exe",
            "GGScanner.exe?other=1",
            "https://example.com/a.exe",
            "a.zip",
        ] {
            let release = ResolvedRelease {
                repository: "Anyrainel/GGScanner",
                tag: "v2026.10.07".into(),
                revision: 125,
                assets: ReleaseAssets {
                    scanner: Some(invalid.into()),
                    capture: None,
                },
                published_assets: None,
            };
            assert!(release.download_url(ASSET_SCANNER).is_err());
        }
    }

    #[test]
    fn redirect_fallback_follows_repository_rename_and_relative_locations() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/GOODScanner/releases/latest",
            server.server_addr()
        );
        let worker =
            std::thread::spawn(move || {
                for (expected, location) in [
                    ("/GOODScanner/releases/latest", "/GGScanner/releases/latest"),
                    ("/GGScanner/releases/latest", "../releases/tag/v2026.10.07"),
                ] {
                    let request = server
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap()
                        .unwrap();
                    assert_eq!(request.url(), expected);
                    request
                        .respond(tiny_http::Response::empty(302).with_header(
                            tiny_http::Header::from_bytes("Location", location).unwrap(),
                        ))
                        .unwrap();
                }
            });
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        assert_eq!(
            follow_release_redirects(&client, &url).as_deref(),
            Some("v2026.10.07")
        );
        worker.join().unwrap();
    }

    #[test]
    fn redirect_fallback_stops_on_redirect_loops() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}/releases/latest", server.server_addr());
        let worker = std::thread::spawn(move || {
            for _ in 0..5 {
                let request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                request
                    .respond(tiny_http::Response::empty(302).with_header(
                        tiny_http::Header::from_bytes("Location", "/releases/latest").unwrap(),
                    ))
                    .unwrap();
            }
        });
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        assert!(follow_release_redirects(&client, &url).is_none());
        worker.join().unwrap();
    }

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("goodscanner-update-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn replacement_preserves_original_as_backup() {
        let dir = test_dir();
        let exe = dir.join("scanner.exe");
        let old = exe.with_extension("exe.old");
        std::fs::write(&exe, b"original").unwrap();
        std::fs::write(&old, b"previous backup").unwrap();
        replace_executable(&exe, &old, b"replacement").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"replacement");
        assert_eq!(std::fs::read(&old).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn locked_old_backup_leaves_current_executable_intact() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = test_dir();
        let exe = dir.join("scanner.exe");
        let old = exe.with_extension("exe.old");
        std::fs::write(&exe, b"original").unwrap();
        std::fs::write(&old, b"locked backup").unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&old)
            .unwrap();
        let error = replace_executable(&exe, &old, b"replacement").unwrap_err();
        assert!(error.to_string().contains("close the previous application"));
        assert_eq!(std::fs::read(&exe).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        drop(lock);
        assert_eq!(std::fs::read(&old).unwrap(), b"locked backup");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn staging_failure_does_not_rename_current_executable() {
        let dir = test_dir();
        let exe = dir.join("scanner.exe");
        let old = exe.with_extension("exe.old");
        let staged = exe.with_extension(format!("exe.{}.new", std::process::id()));
        std::fs::write(&exe, b"original").unwrap();
        std::fs::create_dir(&staged).unwrap();
        assert!(replace_executable(&exe, &old, b"replacement").is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"original");
        assert!(!old.exists());
        assert!(staged.is_dir());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
