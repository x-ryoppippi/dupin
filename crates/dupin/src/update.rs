//! Self-update for the signed release builds.
//!
//! The release packaging writes the channel's feed URL into Info.plist (`DupinUpdateFeed`);
//! a build without one (Nix, `cargo run`) never updates, since Nix owns its store paths. The
//! feed is a small JSON file beside the release's zip. An update is installed only when the
//! downloaded bundle satisfies the running bundle's own designated requirement (same bundle
//! identifier, same Developer ID team) and Gatekeeper accepts it, then swapped in with two
//! renames and launched once the panel is hidden.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rune::AppHandle;
use serde::Deserialize;

use crate::ui::Ui;

/// Long enough after launch not to compete with the first app scan and the panel's warm-up.
const FIRST_CHECK: Duration = Duration::from_secs(60);
const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// The name the release zip gives the bundle, whatever the user renamed the installed copy to.
const ARCHIVED_BUNDLE: &str = "Dupin.app";

static ENABLED: AtomicBool = AtomicBool::new(true);

/// Follow `auto_update` in config.toml; the next check reads it.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// What a channel's `update.json` says about its newest build.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Feed {
    pub version: String,
    /// `CFBundleVersion`: the commit count of the build, which only grows along `main`.
    pub build: u64,
    pub url: String,
    pub sha256: String,
}

impl Feed {
    fn parse(json: &str) -> Result<Feed, String> {
        serde_json::from_str(json).map_err(|e| format!("the update feed is invalid: {e}"))
    }
}

/// The running build, when it can update itself.
struct Installed {
    bundle: PathBuf,
    feed: String,
    build: u64,
    /// The bundle's designated requirement, which an update must satisfy.
    requirement: String,
}

impl Installed {
    fn current() -> Result<Option<Installed>, String> {
        let Some(feed) = dupin_mac::bundle::info_string("DupinUpdateFeed") else {
            return Ok(None);
        };
        let bundle = dupin_mac::bundle::path();
        let build = dupin_mac::bundle::info_string("CFBundleVersion")
            .and_then(|v| v.parse().ok())
            .ok_or("CFBundleVersion is not a build number")?;
        let requirement = designated_requirement(&bundle)?;
        // An ad-hoc signature's requirement is its own hash, which no other build satisfies.
        if !requirement.contains("anchor apple generic") {
            return Err(format!("{} is not signed with a Developer ID", bundle.display()));
        }
        Ok(Some(Installed {
            bundle,
            feed,
            build,
            requirement,
        }))
    }
}

/// Start checking the channel's feed, when this build has one.
pub fn spawn(enabled: bool, panel: AppHandle<Ui>) {
    set_enabled(enabled);
    let installed = match Installed::current() {
        Ok(Some(installed)) => installed,
        Ok(None) => return,
        Err(e) => return crate::log::warn(&format!("updates are off: {e}")),
    };
    std::thread::Builder::new()
        .name("dupin-update".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK);
            loop {
                if ENABLED.load(Ordering::Relaxed) {
                    match check(&installed) {
                        Ok(Some(version)) => {
                            crate::log::warn(&format!("installed Dupin {version}"));
                            let bundle = installed.bundle.clone();
                            panel.update(move |s, _| s.updated(bundle));
                            return;
                        }
                        Ok(None) => {}
                        Err(e) => crate::log::warn(&format!("couldn't update: {e}")),
                    }
                }
                std::thread::sleep(INTERVAL);
            }
        })
        .expect("spawn updater");
}

/// Install the feed's build if it is newer, returning its version.
fn check(installed: &Installed) -> Result<Option<String>, String> {
    let feed = Feed::parse(&output(curl(Duration::from_secs(60)).arg(&installed.feed))?)?;
    if feed.build <= installed.build {
        return Ok(None);
    }
    install(installed, &feed)?;
    Ok(Some(feed.version))
}

fn install(installed: &Installed, feed: &Feed) -> Result<(), String> {
    let parent = installed
        .bundle
        .parent()
        .ok_or("the app bundle has no parent folder")?;
    // Beside the bundle, so the swap is a rename on one volume.
    let staging = parent.join(".dupin-update");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging).map_err(|e| format!("couldn't create {}: {e}", staging.display()))?;
    let result = stage_and_swap(installed, feed, &staging);
    // When neither the update nor the old app is in place, the old app is only in staging.
    if installed.bundle.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

fn stage_and_swap(installed: &Installed, feed: &Feed, staging: &Path) -> Result<(), String> {
    let zip = staging.join("update.zip");
    output(
        curl(Duration::from_secs(15 * 60))
            .arg("--output")
            .arg(&zip)
            .arg(&feed.url),
    )?;

    let sum = output(Command::new("/usr/bin/shasum").args(["-a", "256"]).arg(&zip))?;
    if sum.split_whitespace().next() != Some(feed.sha256.as_str()) {
        return Err("the download doesn't match the feed's checksum".into());
    }

    let unpacked = staging.join("unpacked");
    output(
        Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(&zip)
            .arg(&unpacked),
    )?;
    let new = unpacked.join(ARCHIVED_BUNDLE);

    output(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(format!("-R={}", installed.requirement))
            .arg(&new),
    )
    .map_err(|e| format!("the update isn't signed as this app is: {e}"))?;
    output(
        Command::new("/usr/sbin/spctl")
            .args(["--assess", "--type", "execute"])
            .arg(&new),
    )
    .map_err(|e| format!("Gatekeeper rejects the update: {e}"))?;

    // The feed is unsigned; the bundle's own Info.plist is covered by its signature, so it,
    // not the feed, decides that the update is newer and on this channel.
    let build = info_value(&new, "CFBundleVersion")?
        .parse::<u64>()
        .map_err(|_| "the update's CFBundleVersion is not a build number")?;
    if build <= installed.build {
        return Err(format!(
            "the update is build {build}, not newer than {}",
            installed.build
        ));
    }
    if info_value(&new, "DupinUpdateFeed")? != installed.feed {
        return Err("the update belongs to another channel".into());
    }

    let old = staging.join("old.app");
    std::fs::rename(&installed.bundle, &old).map_err(|e| format!("couldn't move the old app aside: {e}"))?;
    if let Err(e) = std::fs::rename(&new, &installed.bundle) {
        return Err(match std::fs::rename(&old, &installed.bundle) {
            Ok(()) => format!("couldn't move the update into place: {e}"),
            Err(back) => format!(
                "couldn't move the update into place ({e}) or the old app back ({back}); it is at {}",
                old.display()
            ),
        });
    }
    Ok(())
}

/// curl that fails on HTTP errors and gives up on a stalled server.
fn curl(max: Duration) -> Command {
    let mut command = Command::new("/usr/bin/curl");
    command
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--connect-timeout", "30", "--max-time"])
        .arg(max.as_secs().to_string());
    command
}

fn info_value(bundle: &Path, key: &str) -> Result<String, String> {
    let value = output(
        Command::new("/usr/bin/plutil")
            .args(["-extract", key, "raw", "-o", "-"])
            .arg(bundle.join("Contents/Info.plist")),
    )?;
    Ok(value.trim().to_owned())
}

/// Start the installed bundle once this process has exited, and exit.
pub fn relaunch(bundle: &Path) -> ! {
    let spawned = Command::new("/bin/sh")
        .args([
            "-c",
            "while /bin/kill -0 \"$1\" 2>/dev/null; do /bin/sleep 0.1; done; exec /usr/bin/open \"$2\"",
            "sh",
        ])
        .arg(std::process::id().to_string())
        .arg(bundle)
        .spawn();
    if let Err(e) = spawned {
        crate::log::warn(&format!("couldn't relaunch after the update: {e}"));
    }
    std::process::exit(0);
}

fn designated_requirement(bundle: &Path) -> Result<String, String> {
    let out = output(
        Command::new("/usr/bin/codesign")
            .args(["--display", "--requirements", "-"])
            .arg(bundle),
    )?;
    out.lines()
        .find_map(|line| line.trim_start_matches("# ").strip_prefix("designated => "))
        .map(str::to_owned)
        .ok_or_else(|| format!("{} has no designated requirement", bundle.display()))
}

/// Run to completion; stdout on success, stderr as the error otherwise.
fn output(command: &mut Command) -> Result<String, String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let out = command
        .output()
        .map_err(|e| format!("couldn't run {program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_feed() {
        let feed = Feed::parse(
            r#"{"version":"0.2.0","build":42,"url":"https://example.com/Dupin.zip","sha256":"ab"}"#,
        )
        .unwrap();
        assert_eq!(
            feed,
            Feed {
                version: "0.2.0".into(),
                build: 42,
                url: "https://example.com/Dupin.zip".into(),
                sha256: "ab".into(),
            }
        );
    }

    #[test]
    fn rejects_a_feed_without_a_checksum() {
        assert!(Feed::parse(r#"{"version":"0.2.0","build":42,"url":"u"}"#).is_err());
    }
}
