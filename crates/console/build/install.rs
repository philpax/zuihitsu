//! The npm-install decision logic for the console build pipeline, kept free of `main` so it can be
//! unit-tested (build scripts cannot import their own crate; the crate's test module includes this
//! file via `#[path]`).
//!
//! Freshness is decided by directory mtimes, with no marker file: `node_modules` is fresh when it
//! exists and its mtime is not older than `package-lock.json` and `package.json`. Coarse
//! filesystems could theoretically produce equal mtimes (treated as fresh); the conservative
//! direction is a skipped install with a clear failure if a module is genuinely missing, and git
//! checkouts set fresh mtimes on the lockfile anyway.

use std::{fs, path::Path, time::SystemTime};

/// What the pipeline should do about `node_modules`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The tree exists and is at least as new as the lockfile and manifest — build with it as-is.
    Fresh,
    /// The tree is missing or stale — run `npm ci` (respects the lockfile, never writes it).
    InstallNpmCi,
    /// The tree is missing or stale but a Vite dev server is running — do not touch it. `npm ci`
    /// deletes the tree, which would tear down a live HMR session under the dev server.
    SkipDevServerActive,
}

/// Decide what to do with `node_modules`. `node_modules` is `None` when the directory is absent.
pub fn decide(
    node_modules: Option<SystemTime>,
    lock: SystemTime,
    manifest: SystemTime,
    dev_server_active: bool,
) -> Mode {
    let fresh = node_modules.is_some_and(|tree| lock <= tree && manifest <= tree);
    match (fresh, dev_server_active) {
        (true, _) => Mode::Fresh,
        (false, true) => Mode::SkipDevServerActive,
        (false, false) => Mode::InstallNpmCi,
    }
}

/// Whether the console dev server is running, judged by the pidfile the wrapped `npm run dev`
/// script writes into `node_modules/.zuihitsu-vite.pid`. A pidfile whose process is dead counts as
/// inactive and is removed, so a crashed dev server never blocks installs.
pub fn dev_server_active(pidfile: &Path) -> bool {
    let Ok(content) = fs::read_to_string(pidfile) else {
        return false;
    };
    let Ok(pid) = content.trim().parse::<u32>() else {
        return false;
    };
    if process_alive(pid) {
        true
    } else {
        let _ = fs::remove_file(pidfile);
        false
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    // Signal 0 probes for existence without delivering a signal. EPERM (a zombie reparented to
    // init, say) still means the process exists.
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, SYNCHRONIZE},
    };
    // `OpenProcess` with `SYNCHRONIZE` fails when the process does not exist; the handle itself is
    // then enough to prove liveness. `CloseHandle` is unnecessary for a nonexistent process, so
    // only close a handle we actually got.
    let handle = unsafe { OpenProcess(SYNCHRONIZE, false, pid) };
    if handle.is_null() {
        return false;
    }
    unsafe { CloseHandle(handle) };
    true
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    fn times(offset_secs: u64) -> (SystemTime, SystemTime, SystemTime) {
        let now = SystemTime::now();
        let lock = now - Duration::from_secs(offset_secs);
        let manifest = now - Duration::from_secs(offset_secs + 1);
        (now, lock, manifest)
    }

    /// The tree is missing entirely — install.
    #[test]
    fn missing_tree_installs() {
        let (_, lock, manifest) = times(10);
        assert_eq!(decide(None, lock, manifest, false), Mode::InstallNpmCi);
    }

    /// Missing with a dev server running — skip the install anyway: `npm ci` would delete a tree
    /// the dev server is using, and a missing tree under a live server is a broken checkout the
    /// vite build will fail loudly on.
    #[test]
    fn missing_tree_under_dev_server_skips() {
        let (_, lock, manifest) = times(10);
        assert_eq!(
            decide(None, lock, manifest, true),
            Mode::SkipDevServerActive
        );
    }

    /// The tree exists and is at least as new as the lockfile and manifest — build with it.
    #[test]
    fn fresh_tree_is_left_alone() {
        let (tree, lock, _manifest) = times(10);
        assert_eq!(decide(Some(tree), lock, _manifest, false), Mode::Fresh);
        // Freshness wins over the dev server: there is no install to skip.
        assert_eq!(decide(Some(tree), lock, _manifest, true), Mode::Fresh);
    }

    /// A lockfile newer than the tree means the tree is stale — install.
    #[test]
    fn stale_lock_installs() {
        let (_, lock, manifest) = times(10);
        // `package-lock.json` was touched after the tree.
        let tree = lock + Duration::from_secs(5);
        let newer_lock = tree + Duration::from_secs(5);
        assert_eq!(
            decide(Some(tree), newer_lock, manifest, false),
            Mode::InstallNpmCi
        );
        // Boundary: lock mtime exactly equal to the tree mtime counts as fresh.
        assert_eq!(decide(Some(tree), tree, manifest, false), Mode::Fresh);
    }

    /// A stale manifest with a fresh tree — install.
    #[test]
    fn stale_manifest_installs() {
        let (tree, lock, _manifest) = times(10);
        let newer_manifest = tree + Duration::from_secs(5);
        assert_eq!(
            decide(Some(tree), lock, newer_manifest, false),
            Mode::InstallNpmCi
        );
    }

    /// Stale or missing with a live dev server — do not touch the tree.
    #[test]
    fn stale_under_dev_server_skips() {
        let (_, lock, manifest) = times(10);
        let tree = lock + Duration::from_secs(5);
        let newer_lock = tree + Duration::from_secs(5);
        assert_eq!(
            decide(Some(tree), newer_lock, manifest, true),
            Mode::SkipDevServerActive
        );
    }

    fn pidfile_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("zuihitsu-console-test-{}.pid", std::process::id()))
    }

    /// Absent pidfile — no dev server.
    #[test]
    fn absent_pidfile_is_inactive() {
        assert!(!dev_server_active(&pidfile_path()));
    }

    /// A pidfile naming a live process — active (unix; the Windows branch is compile-only here).
    #[test]
    fn live_pid_is_active() {
        let path = pidfile_path();
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn sleep");
        std::fs::write(&path, child.id().to_string()).unwrap();
        assert!(dev_server_active(&path));
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&path);
    }

    /// A pidfile naming a dead process is inactive and removed — self-healing.
    #[test]
    fn dead_pid_is_inactive_and_removed() {
        let path = pidfile_path();
        std::fs::write(&path, "999999999").unwrap();
        assert!(!dev_server_active(&path));
        assert!(!path.exists(), "stale pidfile should be removed");
    }
}
