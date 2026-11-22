use crate::{FontFerryError, Result};
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::Path,
};

/// Advisory OS lock held for one FontFerry operation.
///
/// Keep the descriptor private and do not clone the guard: unlocking is tied to
/// the acquiring process and its operation lifetime, not to the final descriptor
/// reference. Never unlink the lock file, including after this guard is dropped.
#[derive(Debug)]
pub struct OperationGuard {
    file: File,
    owner_pid: u32,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        // On Unix, fork can temporarily retain a descriptor sharing the same
        // open-file description. Merely closing our descriptor can leave its
        // flock held until that child execs/exits. Release explicitly instead.
        // A forked child's copy must never release the parent's active lock.
        if self.owner_pid == std::process::id() {
            let _ = self.file.unlock();
        }
    }
}

pub(crate) fn acquire(path: &Path) -> Result<OperationGuard> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| FontFerryError::State(e.to_string()))?;
    file.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => FontFerryError::OperationBusy,
        TryLockError::Error(error) => {
            FontFerryError::State(format!("cannot lock font operations: {error}"))
        }
    })?;
    Ok(OperationGuard {
        file,
        owner_pid: std::process::id(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    #[test]
    fn drop_explicitly_releases_lock_with_another_descriptor_alive()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let lock = directory.path().join("operation.lock");
        let guard = acquire(&lock)?;
        // An alias models the shared open-file description retained by fork;
        // it is test-only, because the public guard does not expose its File.
        let descriptor_alias = guard.file.try_clone()?;
        assert!(matches!(acquire(&lock), Err(FontFerryError::OperationBusy)));
        drop(guard);
        let next = acquire(&lock)?;
        assert!(acquire(&lock).is_err());
        // Closing the old alias cannot unlock a new operation's separate handle.
        drop(descriptor_alias);
        assert!(acquire(&lock).is_err());
        drop(next);
        let _next = acquire(&lock)?;
        assert!(lock.exists());
        Ok(())
    }

    #[test]
    fn invalid_lock_path_is_not_contention() -> std::result::Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        assert!(matches!(
            acquire(
                &directory
                    .path()
                    .join("missing-parent")
                    .join("operation.lock")
            ),
            Err(FontFerryError::State(_))
        ));
        Ok(())
    }

    #[test]
    fn non_owner_guard_cannot_unlock_the_acquiring_process()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let lock = directory.path().join("operation.lock");
        let guard = acquire(&lock)?;
        let child_copy = OperationGuard {
            file: guard.file.try_clone()?,
            // Deterministically exercise the PID-mismatch branch without unsafe
            // fork inside the multi-threaded Rust test harness.
            owner_pid: std::process::id().wrapping_add(1),
        };
        drop(child_copy);
        assert!(acquire(&lock).is_err());
        drop(guard);
        let _next = acquire(&lock)?;
        Ok(())
    }

    #[test]
    fn subprocess_probe() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let Some(path) = std::env::var_os("FONTFERRY_TEST_LOCK") else {
            return Ok(());
        };
        let mode = std::env::var("FONTFERRY_TEST_LOCK_MODE")?;
        let guard = acquire(Path::new(&path));
        if mode == "busy" {
            assert!(guard.is_err());
            return Ok(());
        }
        let _guard = guard?;
        if mode == "hold" || mode == "release" {
            std::fs::write(Path::new(&path).with_extension("ready"), b"ready")?;
            let _ = std::io::stdin().read(&mut [0])?;
        }
        if mode == "release" {
            drop(_guard);
            std::fs::write(Path::new(&path).with_extension("released"), b"released")?;
            let _ = std::io::stdin().read(&mut [0])?;
        }
        Ok(())
    }

    #[test]
    fn process_can_release_operation_while_remaining_alive()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let lock = directory.path().join("operation.lock");
        let mut child = Command::new(std::env::current_exe()?)
            .args(["--exact", "operation::tests::subprocess_probe"])
            .env("FONTFERRY_TEST_LOCK", &lock)
            .env("FONTFERRY_TEST_LOCK_MODE", "release")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        let check = (|| -> std::result::Result<(), Box<dyn std::error::Error>> {
            for marker in ["ready", "released"] {
                let started = Instant::now();
                while !lock.with_extension(marker).exists() {
                    if child.try_wait()?.is_some() || started.elapsed() > Duration::from_secs(10) {
                        return Err("lock child did not reach expected stage".into());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                if marker == "ready" {
                    if acquire(&lock).is_ok() {
                        return Err("child lock was not exclusive".into());
                    }
                    child
                        .stdin
                        .as_mut()
                        .ok_or("missing child input")?
                        .write_all(b"r")?;
                }
            }
            if child.try_wait()?.is_some() {
                return Err("child exited before the normal-drop check".into());
            }
            let _guard = acquire(&lock)?;
            Ok(())
        })();
        let _ = child.kill();
        child.wait()?;
        check
    }

    #[test]
    fn processes_contend_and_abnormal_exit_releases_lock()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let lock = directory.path().join("operation.lock");
        let executable = std::env::current_exe()?;
        let command = |mode| {
            let mut command = Command::new(&executable);
            command
                .args([
                    "--exact",
                    "operation::tests::subprocess_probe",
                    "--nocapture",
                ])
                .env("FONTFERRY_TEST_LOCK", &lock)
                .env("FONTFERRY_TEST_LOCK_MODE", mode)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit());
            command
        };
        let mut holder = command("hold").stdin(Stdio::piped()).spawn()?;
        let started = Instant::now();
        while !lock.with_extension("ready").exists() {
            if started.elapsed() > Duration::from_secs(10) {
                holder.kill()?;
                holder.wait()?;
                return Err("lock holder did not start".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let busy = command("busy").status()?;
        holder.kill()?;
        holder.wait()?;
        assert!(busy.success());
        assert!(command("free").status()?.success());
        Ok(())
    }
}
