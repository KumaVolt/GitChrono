//! Updates from GitHub releases: check the latest release, download this
//! platform's file, verify it against the release's SHA256SUMS.txt, swap the
//! installed app, relaunch. Network and disk work runs off the UI thread.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, anyhow, bail};
use semver::Version;
use sha2::{Digest as _, Sha256};

pub const REPO: &str = "KumaVolt/GitChrono";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub page: String,
    pub asset_name: String,
    pub asset_url: String,
    pub sums_url: Option<String>,
}

pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is semver")
}

/// The release file built for this OS and CPU (see .github/workflows/release.yml).
pub fn platform_suffix() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => Some("-macos-universal.dmg"),
        ("windows", "x86_64") => Some("-windows-x86_64.zip"),
        ("linux", "x86_64") => Some("-linux-x86_64.tar.gz"),
        ("linux", "aarch64") => Some("-linux-aarch64.tar.gz"),
        _ => None,
    }
}

/// Reads GitHub's "latest release" JSON; `None` when it isn't newer than
/// `current` or has nothing for this platform.
pub fn parse_latest(json: &serde_json::Value, current: &Version, suffix: &str) -> Option<Release> {
    if json["draft"].as_bool() == Some(true) || json["prerelease"].as_bool() == Some(true) {
        return None;
    }
    let version = Version::parse(json["tag_name"].as_str()?.trim_start_matches('v')).ok()?;
    if &version <= current {
        return None;
    }
    let assets = json["assets"].as_array()?;
    let url = |name: &str| assets.iter().find(|a| a["name"].as_str() == Some(name)).and_then(|a| a["browser_download_url"].as_str()).map(str::to_string);
    let asset = assets.iter().find_map(|a| a["name"].as_str().filter(|n| n.ends_with(suffix)))?.to_string();
    Some(Release {
        page: json["html_url"].as_str().unwrap_or_default().to_string(),
        asset_url: url(&asset)?,
        asset_name: asset,
        sums_url: url("SHA256SUMS.txt"),
        version,
    })
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(600))).user_agent(concat!("GitChrono/", env!("CARGO_PKG_VERSION"))).build().into()
}

/// The newest published release, if it's newer than this build.
pub fn check() -> Result<Option<Release>> {
    let Some(suffix) = platform_suffix() else { return Ok(None) };
    let json: serde_json::Value = agent()
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_json()?;
    Ok(parse_latest(&json, &current(), suffix))
}

/// The checksum listed for `name` in a SHA256SUMS.txt (`<hex>  <name>` lines).
pub fn expected_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.to_lowercase())
    })
}

fn downloads_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("gitchrono").join("updates")
}

/// Downloads the release file, reporting progress 0.0–1.0, and checks it
/// against SHA256SUMS.txt.
pub fn download(release: &Release, progress: impl Fn(f32)) -> Result<PathBuf> {
    let dir = downloads_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(&release.asset_name);
    let mut response = agent().get(&release.asset_url).call()?;
    let total = response.body().content_length().unwrap_or(0);
    let mut reader = response.body_mut().as_reader();
    let mut file = std::fs::File::create(&path)?;
    let mut hasher = Sha256::new();
    let (mut buf, mut done) = (vec![0u8; 64 * 1024], 0u64);
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if total > 0 {
            progress(done as f32 / total as f32);
        }
    }
    file.flush()?;
    let actual = hex::encode(hasher.finalize());
    let sums_url = release.sums_url.as_deref().ok_or_else(|| anyhow!("the release has no SHA256SUMS.txt"))?;
    let sums = agent().get(sums_url).call()?.body_mut().read_to_string()?;
    let expected = expected_sum(&sums, &release.asset_name).ok_or_else(|| anyhow!("{} isn't listed in SHA256SUMS.txt", release.asset_name))?;
    if actual != expected {
        let _ = std::fs::remove_file(&path);
        bail!("the download is damaged (checksum mismatch); try again");
    }
    Ok(path)
}

fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("couldn't run {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!("{:?} failed: {}", cmd.get_program(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// What to start after installing.
#[derive(Debug, Clone)]
pub struct Installed(pub PathBuf);

/// Replaces the running app with the downloaded one. The running process
/// keeps working until it's restarted.
pub fn install(download: &Path) -> Result<Installed> {
    install_at(download, &std::env::current_exe()?.canonicalize()?)
}

/// `install` for the app whose executable is at `exe`.
pub fn install_at(download: &Path, exe: &Path) -> Result<Installed> {
    let exe = exe.to_path_buf();
    let work = downloads_dir().join("unpacked");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;

    if cfg!(target_os = "macos") {
        // …/GitChrono.app/Contents/MacOS/gitchrono
        let bundle = exe.ancestors().nth(3).filter(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf).ok_or_else(|| {
            anyhow!("updates install into GitChrono.app, but this copy runs from {}", exe.display())
        })?;
        let mount = work.join("mount");
        run(Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-quiet", "-mountpoint"]).arg(&mount).arg(download))?;
        let staged = bundle.with_extension("app.new");
        let _ = std::fs::remove_dir_all(&staged);
        let copied = run(Command::new("ditto").arg(mount.join("GitChrono.app")).arg(&staged));
        let _ = run(Command::new("hdiutil").args(["detach", "-quiet"]).arg(&mount));
        copied?;
        swap_dir(&bundle, &staged)?;
        return Ok(Installed(bundle));
    }

    // Windows' tar reads zip files too.
    run(Command::new("tar").arg("-xf").arg(download).arg("-C").arg(&work))?;
    let name = if cfg!(windows) { "gitchrono.exe" } else { "gitchrono" };
    let new = find_file(&work, name).ok_or_else(|| anyhow!("{name} not found in the download"))?;
    let staged = exe.with_extension("new");
    std::fs::copy(&new, &staged)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    }
    if cfg!(windows) {
        // A running .exe can't be overwritten but can be renamed; the old one
        // is deleted on the next start (see `cleanup`).
        let old = exe.with_extension("old.exe");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old)?;
    }
    std::fs::rename(&staged, &exe)?;
    Ok(Installed(exe))
}

fn swap_dir(target: &Path, staged: &Path) -> Result<()> {
    let old = target.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(target, &old).with_context(|| format!("can't replace {} (is it writable?)", target.display()))?;
    if let Err(e) = std::fs::rename(staged, target) {
        let _ = std::fs::rename(&old, target);
        return Err(e.into());
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|f| f == name) {
            return Some(path);
        }
    }
    None
}

/// Starts the installed app; the caller quits right after.
pub fn relaunch(installed: &Installed) -> Result<()> {
    if cfg!(target_os = "macos") {
        Command::new("open").arg("-n").arg(&installed.0).spawn()?;
    } else {
        Command::new(&installed.0).spawn()?;
    }
    Ok(())
}

/// Removes what a previous update left behind (the renamed Windows exe).
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old.exe"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "prerelease": prerelease,
            "html_url": "https://github.com/KumaVolt/GitChrono/releases/tag/v0.3.0",
            "assets": [
                { "name": "GitChrono-0.3.0-macos-universal.dmg", "browser_download_url": "https://x/dmg" },
                { "name": "GitChrono-0.3.0-linux-x86_64.tar.gz", "browser_download_url": "https://x/tgz" },
                { "name": "SHA256SUMS.txt", "browser_download_url": "https://x/sums" }
            ]
        })
    }

    #[test]
    fn picks_a_newer_release_for_this_platform() {
        let current = Version::parse("0.2.0").unwrap();
        let r = parse_latest(&release("v0.3.0", false), &current, "-macos-universal.dmg").unwrap();
        assert_eq!((r.version.to_string(), r.asset_url.as_str(), r.sums_url.as_deref()), ("0.3.0".into(), "https://x/dmg", Some("https://x/sums")));
        assert!(parse_latest(&release("v0.2.0", false), &current, "-macos-universal.dmg").is_none(), "same version");
        assert!(parse_latest(&release("v0.3.0", true), &current, "-macos-universal.dmg").is_none(), "pre-release");
        assert!(parse_latest(&release("v0.3.0", false), &current, "-windows-x86_64.zip").is_none(), "no file for this platform");
        assert!(parse_latest(&release("v0.3.0", false), &Version::parse("0.3.0-dev.1").unwrap(), "-linux-x86_64.tar.gz").is_some(), "dev builds update to the release");
    }

    /// Downloads the latest real release, checks it and installs it over a
    /// stand-in GitChrono.app. Needs network: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn downloads_verifies_and_installs_the_latest_release() {
        let suffix = platform_suffix().unwrap();
        let json: serde_json::Value = agent().get(&format!("https://api.github.com/repos/{REPO}/releases/latest")).call().unwrap().body_mut().read_json().unwrap();
        let release = parse_latest(&json, &Version::new(0, 0, 0), suffix).expect("a release for this platform");
        let file = download(&release, |_| {}).unwrap();

        let root = std::env::temp_dir().join(format!("gitchrono-update-{}", std::process::id()));
        let exe = root.join("GitChrono.app/Contents/MacOS/gitchrono");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "old").unwrap();
        let installed = install_at(&file, &exe).unwrap();
        assert_eq!(installed.0, root.join("GitChrono.app"));
        let plist = std::fs::read_to_string(root.join("GitChrono.app/Contents/Info.plist")).unwrap();
        assert!(plist.contains(&format!("<string>{}</string>", release.version)), "the new bundle is in place");
        assert!(!root.join("GitChrono.app.old").exists() && !root.join("GitChrono.app.new").exists(), "no leftovers");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_sha256sums() {
        let sums = "abc123  GitChrono-0.3.0-macos-universal.dmg\nDEF456 *GitChrono-0.3.0-windows-x86_64.zip\n";
        assert_eq!(expected_sum(sums, "GitChrono-0.3.0-macos-universal.dmg").as_deref(), Some("abc123"));
        assert_eq!(expected_sum(sums, "GitChrono-0.3.0-windows-x86_64.zip").as_deref(), Some("def456"));
        assert_eq!(expected_sum(sums, "other"), None);
    }
}
