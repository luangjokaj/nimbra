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
use std::sync::{Arc, Mutex};
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

/// Where each version's release notes are, by its tag.
const RELEASE_NOTES: &str = "https://nimbra.me/release-notes#v";

/// The release notes of `version` on the website.
pub fn release_notes(version: &str) -> String {
    format!("{RELEASE_NOTES}{version}")
}

/// Whether `current` runs for the first time after an update from `last_run`, the version
/// that ran before. Versions before this was kept left none: then it is an update when
/// Nimbra was in use already (`used_before`), and a new install when it was not.
pub fn first_run_after_update(last_run: &str, current: &str, used_before: bool) -> bool {
    if last_run.is_empty() {
        used_before
    } else {
        newer(current, last_run)
    }
}

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

/// How far a new version has got on its way in, for the window to show meanwhile.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub version: String,
    pub stage: Stage,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stage {
    /// Bytes of the disk image so far, and all of them when the server says.
    Downloading { done: u64, total: Option<u64> },
    /// Downloaded: its checksum, signature and notarization are being checked.
    Verifying,
    /// Checked and waiting: it is installed when Nimbra restarts. `check` never reports
    /// this one; the app shows it once `check` says the version is ready.
    Ready,
}

impl Progress {
    /// From 0 to 1; `None` while the size is not known.
    pub fn fraction(&self) -> Option<f32> {
        match self.stage {
            Stage::Downloading { done, total } => total
                .filter(|t| *t > 0)
                .map(|t| (done as f64 / t as f64).clamp(0., 1.) as f32),
            Stage::Verifying | Stage::Ready => Some(1.),
        }
    }
}

/// Where `check` says how far it has got, read from the main thread while it works.
pub type Reporter = Arc<Mutex<Option<Progress>>>;

fn report(reporter: &Reporter, progress: Option<Progress>) {
    if let Ok(mut slot) = reporter.lock() {
        *slot = progress;
    }
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
/// a newer version is waiting to be installed. Blocking: run off the main thread. While it
/// downloads and checks one, it says how far it has got in `reporter` (empty again when it
/// returns).
pub fn check(reporter: &Reporter) -> Result<Option<String>> {
    let result = check_reporting(reporter);
    report(reporter, None);
    result
}

fn check_reporting(reporter: &Reporter) -> Result<Option<String>> {
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
    let result = fetch_and_unpack(&agent, &manifest, &work, reporter);
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

fn fetch_and_unpack(
    agent: &ureq::Agent,
    manifest: &Manifest,
    work: &Path,
    reporter: &Reporter,
) -> Result<()> {
    let progress = |stage| {
        report(
            reporter,
            Some(Progress {
                version: manifest.version.clone(),
                stage,
            }),
        )
    };
    // The disk image, no bigger than a disk image of Nimbra can be.
    let dmg = work.join("Nimbra.dmg");
    progress(Stage::Downloading {
        done: 0,
        total: None,
    });
    let response = agent
        .get(&manifest.url)
        .call()
        .context("downloading the update")?;
    // Only for the progress bar: the size limit and the checksum decide what is kept.
    let total = response
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|t| *t <= MAX_DOWNLOAD);
    let mut reader = response.into_reader().take(MAX_DOWNLOAD + 1);
    let mut bytes = Vec::with_capacity(total.unwrap_or(0) as usize);
    let mut chunk = vec![0; 64 * 1024];
    loop {
        let read = reader.read(&mut chunk).context("downloading the update")?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        progress(Stage::Downloading {
            done: bytes.len() as u64,
            total,
        });
    }
    if bytes.len() as u64 > MAX_DOWNLOAD {
        bail!("the update is too large");
    }
    progress(Stage::Verifying);
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
    fn progress_is_a_fraction_once_the_size_is_known() {
        let at = |done, total| Progress {
            version: "0.1.4".into(),
            stage: Stage::Downloading { done, total },
        };
        assert_eq!(at(0, None).fraction(), None);
        assert_eq!(at(5, Some(0)).fraction(), None);
        assert_eq!(at(25, Some(100)).fraction(), Some(0.25));
        assert_eq!(at(120, Some(100)).fraction(), Some(1.));
        let verifying = Progress {
            version: "0.1.4".into(),
            stage: Stage::Verifying,
        };
        assert_eq!(verifying.fraction(), Some(1.));
    }

    #[test]
    fn release_notes_open_only_on_the_first_run_after_an_update() {
        // Updated from a version that kept the last one that ran.
        assert!(first_run_after_update("0.1.3", "0.1.4", true));
        // The same version again, or an older one put back: no.
        assert!(!first_run_after_update("0.1.4", "0.1.4", true));
        assert!(!first_run_after_update("0.1.5", "0.1.4", true));
        // Nothing kept yet: an update for someone signed in, a new install otherwise.
        assert!(first_run_after_update("", "0.1.4", true));
        assert!(!first_run_after_update("", "0.1.4", false));
        assert_eq!(
            release_notes("0.1.3"),
            "https://nimbra.me/release-notes#v0.1.3"
        );
    }

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
