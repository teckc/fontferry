use crate::{FontFerryError, Result};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

/// Advisory OS lock: never unlink this file, including after the guard is dropped.
pub(crate) fn acquire(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| FontFerryError::State(e.to_string()))?;
    file.try_lock().map_err(|e| {
        FontFerryError::State(format!("另一 FontFerry 进程正在操作字体，请稍后重试：{e}"))
    })?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

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
        if mode == "hold" {
            std::fs::write(Path::new(&path).with_extension("ready"), b"ready")?;
            let _ = std::io::stdin().read(&mut [0])?;
        }
        Ok(())
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
