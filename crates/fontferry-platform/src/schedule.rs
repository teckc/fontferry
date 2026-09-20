use std::{path::Path, process::Command};

use fontferry_core::{FontFerryError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerKind {
    WindowsTaskScheduler,
    MacosLaunchAgent,
    SystemdUser,
}

#[derive(Clone, Debug)]
pub struct ScheduleResult {
    pub kind: SchedulerKind,
    pub enabled: bool,
    pub detail: String,
}

pub fn install_daily_schedule(executable: &Path) -> Result<ScheduleResult> {
    platform::install(executable)
}

pub fn remove_daily_schedule() -> Result<ScheduleResult> {
    platform::remove()
}

/// Both GUI and CLI enter here. A persisted intent precedes native mutation;
/// an interrupted/failed change is unknown until explicitly retried, never a stale success.
pub fn update_daily_schedule(
    engine: &fontferry_core::FontEngine,
    state: &crate::SqliteState,
    executable: &Path,
    enabled: bool,
) -> Result<ScheduleResult> {
    let _guard = engine.acquire_operation_lock()?;
    apply_schedule(state, enabled, || {
        if enabled {
            platform::install(executable)
        } else {
            platform::remove()
        }
    })
}

fn apply_schedule(
    state: &crate::SqliteState,
    enabled: bool,
    apply: impl FnOnce() -> Result<ScheduleResult>,
) -> Result<ScheduleResult> {
    state.set_setting("schedule-pending", &Some(enabled))?;
    let result = apply()?;
    if result.enabled != enabled {
        return Err(FontFerryError::State(
            "scheduler result disagrees with requested state; retry schedule settings".into(),
        ));
    }
    state.set_setting("schedule-enabled", &result.enabled)?;
    state.set_setting("schedule-pending", &Option::<bool>::None)?;
    Ok(result)
}

/// None means a previous operation may have partially changed the native scheduler.
pub fn saved_schedule_state(state: &crate::SqliteState) -> Result<Option<bool>> {
    if state
        .get_setting::<Option<bool>>("schedule-pending")?
        .flatten()
        .is_some()
    {
        return Ok(None);
    }
    Ok(Some(
        state.get_setting("schedule-enabled")?.unwrap_or(false),
    ))
}

fn command_error(error: std::io::Error) -> FontFerryError {
    FontFerryError::Platform(error.to_string())
}

fn checked_output(command: &mut Command) -> Result<std::process::Output> {
    let output = command.output().map_err(command_error)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(FontFerryError::Platform(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

fn run(command: &mut Command) -> Result<()> {
    checked_output(command).map(|_| ())
}

#[cfg(windows)]
mod platform {
    use std::{path::Path, process::Command};

    use fontferry_core::Result;

    use super::{ScheduleResult, SchedulerKind, run};

    const TASK_NAME: &str = r"FontFerry\UserDailyUpdate";

    pub fn install(executable: &Path) -> Result<ScheduleResult> {
        let task = format!("\"{}\" update --eligible --headless", executable.display());
        run(Command::new("schtasks").args([
            "/Create", "/F", "/SC", "DAILY", "/ST", "09:00", "/TN", TASK_NAME, "/TR", &task,
        ]))?;
        Ok(ScheduleResult {
            kind: SchedulerKind::WindowsTaskScheduler,
            enabled: true,
            detail: TASK_NAME.into(),
        })
    }

    pub fn remove() -> Result<ScheduleResult> {
        // Enumerating all tasks distinguishes absence from a failed task query without localized error parsing.
        run(Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", r"$ErrorActionPreference = 'Stop'; Get-ScheduledTask -ErrorAction Stop | Where-Object { $_.TaskName -eq 'UserDailyUpdate' -and $_.TaskPath -eq '\FontFerry\' } | Unregister-ScheduledTask -Confirm:$false -ErrorAction Stop"]))?;
        Ok(ScheduleResult {
            kind: SchedulerKind::WindowsTaskScheduler,
            enabled: false,
            detail: TASK_NAME.into(),
        })
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::{env, fs, path::Path, process::Command};

    use fontferry_core::{FontFerryError, Result};

    use super::{ScheduleResult, SchedulerKind, checked_output, command_error, run};

    const LABEL: &str = "io.github.teckc.fontferry.update";

    pub fn install(executable: &Path) -> Result<ScheduleResult> {
        let home = env::var_os("HOME")
            .ok_or_else(|| FontFerryError::Platform("HOME is not available".into()))?;
        let directory = Path::new(&home).join("Library").join("LaunchAgents");
        fs::create_dir_all(&directory).map_err(command_error)?;
        let plist = directory.join(format!("{LABEL}.plist"));
        let executable = xml_escape(&executable.to_string_lossy());
        let body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>{executable}</string><string>update</string><string>--eligible</string><string>--headless</string></array>
<key>StartCalendarInterval</key><dict><key>Hour</key><integer>9</integer></dict>
<key>RunAtLoad</key><true/>
</dict></plist>
"#
        );
        let user_id = Command::new("id")
            .arg("-u")
            .output()
            .map_err(command_error)?;
        if !user_id.status.success() {
            return Err(FontFerryError::Platform(
                "cannot determine macOS user id".into(),
            ));
        }
        let domain = format!("gui/{}", String::from_utf8_lossy(&user_id.stdout).trim());
        if loaded()? {
            run(Command::new("launchctl")
                .arg("bootout")
                .arg(format!("{domain}/{LABEL}")))?;
        }
        fs::write(&plist, body).map_err(command_error)?;
        run(Command::new("launchctl").args(["bootstrap", &domain, &plist.to_string_lossy()]))?;
        Ok(ScheduleResult {
            kind: SchedulerKind::MacosLaunchAgent,
            enabled: true,
            detail: plist.display().to_string(),
        })
    }

    pub fn remove() -> Result<ScheduleResult> {
        let home = env::var_os("HOME")
            .ok_or_else(|| FontFerryError::Platform("HOME is not available".into()))?;
        let plist = Path::new(&home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LABEL}.plist"));
        if loaded()? {
            let user_id = checked_output(Command::new("id").arg("-u"))?;
            let domain = format!("gui/{}", String::from_utf8_lossy(&user_id.stdout).trim());
            run(Command::new("launchctl")
                .arg("bootout")
                .arg(format!("{domain}/{LABEL}")))?;
        }
        if plist.exists() {
            fs::remove_file(&plist).map_err(command_error)?;
        }
        Ok(ScheduleResult {
            kind: SchedulerKind::MacosLaunchAgent,
            enabled: false,
            detail: plist.display().to_string(),
        })
    }

    fn loaded() -> Result<bool> {
        let output = checked_output(Command::new("launchctl").arg("list"))?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.split_whitespace().last() == Some(LABEL)))
    }

    fn xml_escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::{env, fs, path::Path, process::Command};

    use fontferry_core::{FontFerryError, Result};

    use super::{ScheduleResult, SchedulerKind, checked_output, command_error, run};

    const SERVICE: &str = "fontferry-update.service";
    const TIMER: &str = "fontferry-update.timer";

    pub fn install(executable: &Path) -> Result<ScheduleResult> {
        run(Command::new("systemctl").args(["--user", "show-environment"]))?;
        let home = env::var_os("HOME")
            .ok_or_else(|| FontFerryError::Platform("HOME is not available".into()))?;
        let directory = Path::new(&home)
            .join(".config")
            .join("systemd")
            .join("user");
        fs::create_dir_all(&directory).map_err(command_error)?;
        let command = systemd_escape(&executable.to_string_lossy());
        fs::write(
            directory.join(SERVICE),
            format!(
                "[Unit]\nDescription=Check FontFerry font updates\n\n[Service]\nType=oneshot\nExecStart={command} update --eligible --headless\n"
            ),
        )
        .map_err(command_error)?;
        fs::write(
            directory.join(TIMER),
            "[Unit]\nDescription=Daily FontFerry update check\n\n[Timer]\nOnCalendar=daily\nPersistent=true\nRandomizedDelaySec=30m\n\n[Install]\nWantedBy=timers.target\n",
        )
        .map_err(command_error)?;
        run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        run(Command::new("systemctl").args(["--user", "enable", "--now", TIMER]))?;
        Ok(ScheduleResult {
            kind: SchedulerKind::SystemdUser,
            enabled: true,
            detail: TIMER.into(),
        })
    }

    pub fn remove() -> Result<ScheduleResult> {
        run(Command::new("systemctl").args(["--user", "show-environment"]))?;
        let listed = checked_output(Command::new("systemctl").args([
            "--user",
            "list-unit-files",
            TIMER,
            "--no-legend",
            "--no-pager",
        ]))?;
        if !String::from_utf8_lossy(&listed.stdout).trim().is_empty() {
            run(Command::new("systemctl").args(["--user", "disable", "--now", TIMER]))?;
        }
        Ok(ScheduleResult {
            kind: SchedulerKind::SystemdUser,
            enabled: false,
            detail: TIMER.into(),
        })
    }

    fn systemd_escape(value: &str) -> String {
        format!(
            "\"{}\"",
            value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('%', "%%")
                .replace('$', "$$")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
    fn success(enabled: bool) -> ScheduleResult {
        ScheduleResult {
            kind: SchedulerKind::SystemdUser,
            enabled,
            detail: "test".into(),
        }
    }

    #[test]
    fn partial_native_failure_survives_restart_as_unknown_and_can_retry() -> TestResult {
        let root = tempfile::tempdir()?;
        let database = root.path().join("state.sqlite");
        let state = crate::SqliteState::open(&database)?;
        apply_schedule(&state, true, || Ok(success(true)))?;
        assert!(
            apply_schedule(&state, false, || Err(FontFerryError::Platform(
                "injected failure after unload".into()
            )))
            .is_err()
        );
        drop(state);
        let state = crate::SqliteState::open(&database)?;
        assert_eq!(saved_schedule_state(&state)?, None);
        apply_schedule(&state, false, || Ok(success(false)))?;
        assert_eq!(saved_schedule_state(&state)?, Some(false));
        Ok(())
    }

    #[test]
    fn persistence_failure_after_native_change_never_reports_saved_success() -> TestResult {
        let root = tempfile::tempdir()?;
        let database = root.path().join("state.sqlite");
        let state = crate::SqliteState::open(&database)?;
        let sql = rusqlite::Connection::open(&database)?;
        sql.execute_batch("CREATE TRIGGER fail_schedule BEFORE INSERT ON settings WHEN NEW.key = 'schedule-enabled' BEGIN SELECT RAISE(FAIL, 'injected commit failure'); END;")?;
        let called = std::cell::Cell::new(false);
        assert!(
            apply_schedule(&state, true, || {
                called.set(true);
                Ok(success(true))
            })
            .is_err()
        );
        assert!(called.get());
        assert_eq!(saved_schedule_state(&state)?, None);
        sql.execute_batch("DROP TRIGGER fail_schedule;")?;
        apply_schedule(&state, true, || Ok(success(true)))?;
        assert_eq!(saved_schedule_state(&state)?, Some(true));
        Ok(())
    }

    #[test]
    fn intent_failure_prevents_native_mutation() -> TestResult {
        let root = tempfile::tempdir()?;
        let database = root.path().join("state.sqlite");
        let state = crate::SqliteState::open(&database)?;
        let sql = rusqlite::Connection::open(&database)?;
        sql.execute_batch("CREATE TRIGGER fail_intent BEFORE INSERT ON settings BEGIN SELECT RAISE(FAIL, 'injected intent failure'); END;")?;
        let called = std::cell::Cell::new(false);
        assert!(
            apply_schedule(&state, true, || {
                called.set(true);
                Ok(success(true))
            })
            .is_err()
        );
        assert!(!called.get());
        Ok(())
    }
}
