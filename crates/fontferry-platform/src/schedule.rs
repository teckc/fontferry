use std::{path::Path, process::Command};

use fontferry_core::{FontFerryError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerKind {
    WindowsTaskScheduler,
    MacosLaunchAgent,
    SystemdUser,
    StartupFallback,
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

fn command_error(error: std::io::Error) -> FontFerryError {
    FontFerryError::Platform(error.to_string())
}

fn run(command: &mut Command) -> Result<()> {
    let output = command.output().map_err(command_error)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(FontFerryError::Platform(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

#[cfg(windows)]
mod platform {
    use std::{path::Path, process::Command};

    use fontferry_core::Result;

    use super::{ScheduleResult, SchedulerKind, run};

    const TASK_NAME: &str = r"FontFerry\UserDailyUpdate";

    pub fn install(executable: &Path) -> Result<ScheduleResult> {
        let task = format!(
            "\"{}\" update --eligible --headless",
            executable.display()
        );
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
        let _ignored = Command::new("schtasks")
            .args(["/Delete", "/F", "/TN", TASK_NAME])
            .output();
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

    use super::{ScheduleResult, SchedulerKind, command_error, run};

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
        fs::write(&plist, body).map_err(command_error)?;
        let user_id = Command::new("id")
            .arg("-u")
            .output()
            .map_err(command_error)?;
        if !user_id.status.success() {
            return Err(FontFerryError::Platform("cannot determine macOS user id".into()));
        }
        let domain = format!("gui/{}", String::from_utf8_lossy(&user_id.stdout).trim());
        let _ignored = Command::new("launchctl")
            .args(["bootout", &domain, &plist.to_string_lossy()])
            .output();
        run(Command::new("launchctl").args([
            "bootstrap",
            &domain,
            &plist.to_string_lossy(),
        ]))?;
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
        let _ignored = fs::remove_file(&plist);
        Ok(ScheduleResult {
            kind: SchedulerKind::MacosLaunchAgent,
            enabled: false,
            detail: plist.display().to_string(),
        })
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

    use super::{ScheduleResult, SchedulerKind, command_error, run};

    const SERVICE: &str = "fontferry-update.service";
    const TIMER: &str = "fontferry-update.timer";

    pub fn install(executable: &Path) -> Result<ScheduleResult> {
        if Command::new("systemctl")
            .args(["--user", "--version"])
            .output()
            .is_err()
        {
            return Ok(ScheduleResult {
                kind: SchedulerKind::StartupFallback,
                enabled: true,
                detail: "systemd user session unavailable; application startup fallback enabled"
                    .into(),
            });
        }
        let home = env::var_os("HOME")
            .ok_or_else(|| FontFerryError::Platform("HOME is not available".into()))?;
        let directory = Path::new(&home).join(".config").join("systemd").join("user");
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
        let _ignored = Command::new("systemctl")
            .args(["--user", "disable", "--now", TIMER])
            .output();
        Ok(ScheduleResult {
            kind: SchedulerKind::SystemdUser,
            enabled: false,
            detail: TIMER.into(),
        })
    }

    fn systemd_escape(value: &str) -> String {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}
