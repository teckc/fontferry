use super::*;

#[derive(Debug, Parser)]
#[command(
    name = "fontferry",
    version,
    about = "Cross-platform font update manager"
)]
pub(super) struct Cli {
    #[command(subcommand)]
    pub(super) command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
pub(super) enum CliCommand {
    Check(CheckArgs),
    Update(UpdateArgs),
    Doctor,
    Schedule(ScheduleArgs),
}

#[derive(Debug, Args)]
pub(super) struct CheckArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub(super) struct UpdateArgs {
    #[arg(long)]
    eligible: bool,
    #[arg(long)]
    headless: bool,
}

#[derive(Debug, Args)]
pub(super) struct ScheduleArgs {
    #[arg(long, conflicts_with = "disable")]
    enable: bool,
    #[arg(long, conflicts_with = "enable")]
    disable: bool,
}

pub(super) async fn run_cli(command: CliCommand) -> Result<()> {
    let state = create_state()?;
    state.engine.recover().await?;
    match command {
        CliCommand::Check(arguments) => {
            let statuses = check_all(&state.engine).await;
            if arguments.json {
                println!("{}", serde_json::to_string_pretty(&statuses)?);
            } else {
                for status in statuses {
                    match status {
                        Ok(status) => println!(
                            "{}: {} -> {}{}",
                            status.font_id,
                            status.current_version.as_deref().unwrap_or("not installed"),
                            status.available_version.as_deref().unwrap_or("unknown"),
                            if status.update_available {
                                " (update)"
                            } else {
                                ""
                            }
                        ),
                        Err(error) => eprintln!("{error}"),
                    }
                }
            }
        }
        CliCommand::Update(arguments) => {
            if !arguments.eligible {
                anyhow::bail!("update requires --eligible");
            }
            let installed_by_id: std::collections::HashMap<_, _> = state
                .state
                .list_installed()
                .await?
                .into_iter()
                .map(|font| (font.font_id.clone(), font))
                .collect();
            let mut failures = 0_u32;
            let mut reminders = Vec::new();
            for font in state.engine.fonts() {
                let managed = installed_by_id.contains_key(&font.id);
                let observed = state.state.get_observed(&font.id).await?.is_some();
                if !managed && !observed {
                    continue;
                }
                let status = match state.engine.check_font(&font.id).await {
                    Ok(status) => status,
                    Err(error) => {
                        failures += 1;
                        eprintln!("{}: {error}", font.id);
                        state
                            .engine
                            .record_failure(&font.id, &error.to_string())
                            .await?;
                        continue;
                    }
                };
                if !status.update_available || status.current_version.is_none() {
                    continue;
                }
                if status.delivery_policy == DeliveryPolicy::AutoInstall {
                    match state.engine.update_installed(&font.id).await {
                        Ok(Some(installed)) => {
                            println!("{} -> {}", installed.font_id, installed.version)
                        }
                        Ok(None) => {}
                        Err(error) => {
                            failures += 1;
                            eprintln!("{}: {error}", font.id);
                        }
                    }
                } else {
                    println!("{}: update available (notification only)", font.id);
                    reminders.push(font.name);
                }
            }
            if arguments.headless && !reminders.is_empty() {
                let summary = format!("{} 个字体有可用更新", reminders.len());
                let body = reminders.join("、");
                let _notification_result = notify_rust::Notification::new()
                    .summary(&summary)
                    .body(&body)
                    .appname("FontFerry")
                    .show();
            }
            if failures > 0 {
                if arguments.headless {
                    let _notification_result = notify_rust::Notification::new()
                        .summary("FontFerry 更新失败")
                        .body(&format!(
                            "{failures} 个更新操作失败，请打开字渡的“记录”查看"
                        ))
                        .appname("FontFerry")
                        .show();
                }
                anyhow::bail!("{failures} update operation(s) failed");
            }
        }
        CliCommand::Doctor => {
            println!("data: {}", state.paths.data.display());
            println!("database: {}", state.paths.database().display());
            println!("catalog fonts: {}", state.engine.fonts().len());
            println!("platform: {}", std::env::consts::OS);
        }
        CliCommand::Schedule(arguments) => {
            let _enable_requested = arguments.enable;
            let executable = std::env::current_exe().context("locate executable")?;
            let result = tokio::task::spawn_blocking(move || {
                update_daily_schedule(&state.engine, &state.state, &executable, !arguments.disable)
            })
            .await??;
            println!("{result:?}");
        }
    }
    Ok(())
}
