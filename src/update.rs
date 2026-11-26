//! Nimbra's own updates. A release build checks `nimbra.me/releases/latest.json` a minute
//! after it opens and then every few hours. When there is a newer version it downloads the
//! disk image, checks it, and keeps the new Nimbra.app ready in its caches; the sidebar then
//! offers a restart, and the new app takes the old one's place when Nimbra quits.
//!
//! Nothing in the manifest is trusted on its own. The disk image must match its SHA-256,
//! and the app in it must carry Nimbra's Developer ID signature (this team's, not any
//! developer's), pass Gatekeeper's notarization check, and be the version the manifest
//! names. Only then is it kept.
//!
//! A build without `NIMBRA_UPDATE_FEED` (any build that isn't a release) never updates.
//! Settings › Privacy & Security › Update automatically turns the checks off.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

/// Where release builds read the newest version from (set by `scripts/bundle-macos.sh`).
const FEED: Option<&str> = option_env!("NIMBRA_UPDATE_FEED");
/// Only downloads from Nimbra's own GitHub releases are fetched.
const DOWNLOADS: &str = "https://github.com/luangjokaj/nimbra/releases/download/";
/// The signature an update must carry: Apple's Developer ID chain, this team's leaf.
const REQUIREMENT: &str = "anchor apple generic and certificate leaf[subject.OU] = \"62TSR9B3L6\"";
/// A disk image bigger than this is not Nimbra.
const MAX_DOWNLOAD: u64 = 300 * 1024 * 1024;

/// First check, a moment after launch; then this often.
pub const FIRST: Duration = Duration::from_secs(60);
pub const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// The newest release, as the website serves it.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Manifest {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub notes: String,
}

/// Whether this build can update itself at all.
pub fn enabled() -> bool {
    FEED.is_some() && bundle().is_some()
}

/// The installed Nimbra.app this process runs from.
pub fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    // …/Nimbra.app/Contents/MacOS/nimbra
    let app = exe.parent()?.parent()?.parent()?.to_path_buf();
    (app.extension().is_some_and(|e| e == "app")).then_some(app)
}

/// Where a downloaded update waits: `~/Library/Caches/Nimbra/update`.
fn work_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("Library/Caches/Nimbra/update"))
}

/// The new app, once it is downloaded and checked.
fn staged_app() -> Option<PathBuf> {
    Some(work_dir()?.join("Nimbra.app"))
}

/// The version waiting in the caches, if one is (and it is newer than this one).
pub fn staged() -> Option<String> {
    let app = staged_app()?;
    let version = version_of(&app)?;
    newer(&version, env!("CARGO_PKG_VERSION")).then_some(version)
}

/// Whether `a` is a newer `x.y.z` than `b`.
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Option<(u64, u64, u64)> {
        let mut parts = v.trim().split('.').map(|p| p.parse::<u64>().ok());
        Some((parts.next()??, parts.next()??, parts.next()??))
    };
    matches!((parse(a), parse(b)), (Some(a), Some(b)) if a > b)
}

/// Checks the manifest only what the app expects, from where it expects it.
fn checked(manifest: Manifest) -> Result<Manifest> {
    let numbers = manifest.version.split('.').count() == 3
        && manifest
            .version
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if !numbers {
        bail!("the update's version is not a version");
    }
    if !manifest.url.starts_with(DOWNLOADS) || manifest.url.contains("..") {
        bail!("the update is not from Nimbra's releases");
    }
    if manifest.sha256.len() != 64 || !manifest.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("the update has no valid checksum");
    }
    Ok(manifest)
}

/// Looks for a newer version and, if there is one, gets it ready. `Ok(Some(version))` when
/// a newer version is waiting to be installed. Blocking: run off the main thread.
pub fn check() -> Result<Option<String>> {
    let feed = std::env::var("NIMBRA_UPDATE_FEED")
        .ok()
        .or(FEED.map(str::to_string))
        .ok_or_else(|| anyhow!("this build does not update itself"))?;
    let install = bundle().ok_or_else(|| anyhow!("not running from Nimbra.app"))?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .build();
    let manifest: Manifest = agent
        .get(&feed)
        .call()
        .context("asking for the newest version")?
        .into_json()
        .context("reading the newest version")?;
    let manifest = checked(manifest)?;
    if !newer(&manifest.version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    if staged().as_deref() == Some(manifest.version.as_str()) {
        return Ok(Some(manifest.version));
    }
    // The new app replaces the old one in its folder: that folder must be writable.
    let folder = install.parent().ok_or_else(|| anyhow!("no folder"))?;
    if !writable(folder) {
        bail!("Nimbra can't update itself in {}", folder.display());
    }
    let work = work_dir().ok_or_else(|| anyhow!("no home folder"))?;
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).context("making the update folder")?;
    let result = fetch_and_unpack(&agent, &manifest, &work);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&work);
    }
    result.map(|()| Some(manifest.version))
}

fn writable(folder: &Path) -> bool {
    let probe = folder.join(".nimbra-update-probe");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn fetch_and_unpack(agent: &ureq::Agent, manifest: &Manifest, work: &Path) -> Result<()> {
    // The disk image, no bigger than a disk image of Nimbra can be.
    let dmg = work.join("Nimbra.dmg");
    let response = agent
        .get(&manifest.url)
        .call()
        .context("downloading the update")?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_DOWNLOAD + 1)
        .read_to_end(&mut bytes)
        .context("downloading the update")?;
    if bytes.len() as u64 > MAX_DOWNLOAD {
        bail!("the update is too large");
    }
    let sum = Sha256::digest(&bytes);
    let hex: String = sum.iter().map(|b| format!("{b:02x}")).collect();
    if hex != manifest.sha256.to_ascii_lowercase() {
        bail!("the update is damaged (checksum mismatch)");
    }
    std::fs::write(&dmg, &bytes).context("saving the update")?;

    // Its app, copied out of the mounted image.
    let mount = work.join("mount");
    std::fs::create_dir_all(&mount)?;
    run(Command::new("/usr/bin/hdiutil")
        .args([
            "attach",
            "-nobrowse",
            "-readonly",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(&mount)
        .arg(&dmg))?;
    let staged = work.join("Nimbra.app");
    let copied = run(Command::new("/usr/bin/ditto")
        .arg(mount.join("Nimbra.app"))
        .arg(&staged));
    let _ = run(Command::new("/usr/bin/hdiutil")
        .args(["detach", "-quiet", "-force"])
        .arg(&mount));
    let _ = std::fs::remove_dir(&mount);
    copied?;
    let _ = std::fs::remove_file(&dmg);

    // Signed by this team, notarized, and the version it says it is.
    run(Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "-R"])
        .arg(format!("={REQUIREMENT}"))
        .arg(&staged))
    .context("the update is not signed by Nimbra")?;
    run(Command::new("/usr/sbin/spctl")
        .args(["--assess", "--type", "execute"])
        .arg(&staged))
    .context("the update is not notarized")?;
    if version_of(&staged).as_deref() != Some(manifest.version.as_str()) {
        bail!("the update is not version {}", manifest.version);
    }
    Ok(())
}

/// The version in an app's Info.plist.
fn version_of(app: &Path) -> Option<String> {
    let output = Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(app.join("Contents/Info"))
        .arg("CFBundleShortVersionString")
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|v| !v.is_empty())
}

fn run(command: &mut Command) -> Result<()> {
    let output = command
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {:?}", command.get_program()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "{:?} failed: {}",
            command.get_program(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Once this process has quit, puts the waiting app in the installed one's place (keeping
/// the old one until the new one is there), and opens it again if `relaunch`. Does nothing
/// when no update is waiting.
pub fn install_on_quit(relaunch: bool) {
    let (Some(new), Some(app)) = (staged_app(), bundle()) else {
        return;
    };
    if staged().is_none() {
        return;
    }
    let old = app.with_extension("app.old");
    // Paths go in as arguments, never into the script's text.
    let script = r#"
pid="$1"; new="$2"; app="$3"; old="$4"; relaunch="$5"
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
rm -rf "$old"
if mv "$app" "$old"; then
  if mv "$new" "$app"; then
    rm -rf "$old"
  else
    mv "$old" "$app"
  fi
fi
[ "$relaunch" = 1 ] && /usr/bin/open "$app"
exit 0
"#;
    let _ = Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg("nimbra-update")
        .arg(std::process::id().to_string())
        .arg(&new)
        .arg(&app)
        .arg(&old)
        .arg(if relaunch { "1" } else { "0" })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str, url: &str, sha: &str) -> Manifest {
        Manifest {
            version: version.into(),
            url: url.into(),
            sha256: sha.into(),
            notes: "https://github.com/luangjokaj/nimbra/releases/tag/v0.1.1".into(),
        }
    }

    const SHA: &str = "d0d8030a50fa031a48be09b83a7d431be96e9d6581c54e85a04cc4da604fc81a";

    #[test]
    fn versions_compare_as_numbers() {
        assert!(newer("0.1.10", "0.1.9"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.2.0"));
        assert!(!newer("0.1", "0.0.1"));
        assert!(!newer("x.y.z", "0.0.1"));
    }

    #[test]
    fn only_nimbras_own_releases_are_fetched() {
        let good = format!("{DOWNLOADS}v0.1.1/Nimbra-0.1.1.dmg");
        assert!(checked(manifest("0.1.1", &good, SHA)).is_ok());
        for url in [
            "https://example.com/Nimbra.dmg",
            "http://github.com/luangjokaj/nimbra/releases/download/v0.1.1/Nimbra.dmg",
            "https://github.com/someone/nimbra/releases/download/v0.1.1/Nimbra.dmg",
            "https://github.com/luangjokaj/nimbra/releases/download/../../x/Nimbra.dmg",
        ] {
            assert!(checked(manifest("0.1.1", url, SHA)).is_err(), "{url}");
        }
        assert!(checked(manifest("0.1.1-beta", &good, SHA)).is_err());
        assert!(checked(manifest("0.1.1", &good, "abc")).is_err());
    }
}
