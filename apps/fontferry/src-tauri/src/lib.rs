use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use fontferry_core::{
    DeliveryPolicy, FontDefinition, FontEngine, InstallRequest, InstalledFont, StateRepository,
    UpdateStatus,
};
use fontferry_platform::{
    AppPaths, HttpClient, PlatformFontInstaller, SafeFontPreparer, SqliteState,
    install_daily_schedule, remove_daily_schedule,
};
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

const CATALOG_JSON: &str = include_str!("../../../../catalog/builtin/catalog.json");

#[derive(Debug, Parser)]
#[command(name = "fontferry", version, about = "Cross-platform font update manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    Check(CheckArgs),
    Update(UpdateArgs),
    Doctor,
    Schedule(ScheduleArgs),
}

#[derive(Debug, Args)]
struct CheckArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct UpdateArgs {
    #[arg(long)]
    eligible: bool,
    #[arg(long)]
    headless: bool,
}

#[derive(Debug, Args)]
struct ScheduleArgs {
    #[arg(long, conflicts_with = "disable")]
    enable: bool,
    #[arg(long, conflicts_with = "enable")]
    disable: bool,
}

pub struct AppState {
    engine: Arc<FontEngine>,
    state: Arc<SqliteState>,
    paths: AppPaths,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppState")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Dashboard {
    fonts: Vec<FontDefinition>,
    installed: Vec<InstalledFont>,
    statuses: Vec<UpdateStatus>,
    activities: Vec<fontferry_core::Activity>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallInput {
    font_id: String,
    version: Option<String>,
    variant_ids: Vec<String>,
    accept_license: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct ScheduleInput {
    enabled: bool,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Some(command) = cli.command {
        let runtime = tokio::runtime::Runtime::new().context("create Tokio runtime")?;
        return runtime.block_on(run_cli(command));
    }
    run_gui()
}

async fn run_cli(command: CliCommand) -> Result<()> {
    let state = create_state()?;
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
                            if status.update_available { " (update)" } else { "" }
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
            let installed = state.state.list_installed().await?;
            let mut failures = 0_u32;
            for item in installed {
                let status = match state.engine.check_font(&item.font_id).await {
                    Ok(status) => status,
                    Err(error) => {
                        failures += 1;
                        eprintln!("{}: {error}", item.font_id);
                        continue;
                    }
                };
                if status.update_available && status.delivery_policy == DeliveryPolicy::AutoInstall {
                    let request = InstallRequest {
                        font_id: item.font_id.clone(),
                        version: status.available_version,
                        variant_ids: item.variant_ids,
                        accept_license: false,
                    };
                    match state.engine.install(request).await {
                        Ok(installed) => println!("{} -> {}", installed.font_id, installed.version),
                        Err(error) => {
                            failures += 1;
                            eprintln!("{}: {error}", item.font_id);
                        }
                    }
                } else if status.update_available {
                    println!("{}: update available (notification only)", item.font_id);
                }
            }
            if failures > 0 {
                anyhow::bail!("{failures} update operation(s) failed");
            }
            let _headless = arguments.headless;
        }
        CliCommand::Doctor => {
            println!("data: {}", state.paths.data.display());
            println!("database: {}", state.paths.database().display());
            println!("catalog fonts: {}", state.engine.fonts().len());
            println!("platform: {}", std::env::consts::OS);
        }
        CliCommand::Schedule(arguments) => {
            let _enable_requested = arguments.enable;
            if arguments.disable {
                println!("{:?}", remove_daily_schedule()?);
            } else {
                let executable = std::env::current_exe().context("locate executable")?;
                println!("{:?}", install_daily_schedule(&executable)?);
            }
        }
    }
    Ok(())
}

fn run_gui() -> Result<()> {
    let state = create_state()?;
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ignored = window.show();
                let _ignored = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            dashboard,
            check_font,
            install_font,
            uninstall_font,
            rollback_font,
            save_source,
            set_schedule
        ])
        .run(tauri::generate_context!())
        .context("run Tauri application")
}

fn create_state() -> Result<AppState> {
    let paths = AppPaths::discover()?;
    let catalog: fontferry_core::Catalog =
        serde_json::from_str(CATALOG_JSON).context("parse built-in catalog")?;
    catalog.validate()?;
    let mut fonts = catalog.fonts;
    let state = Arc::new(SqliteState::open(&paths.database())?);
    fonts.extend(state.list_user_sources()?);
    let http = Arc::new(HttpClient::new()?);
    let engine = Arc::new(FontEngine::new(
        fonts,
        http.clone(),
        http,
        Arc::new(SafeFontPreparer),
        Arc::new(PlatformFontInstaller::new(paths.clone())),
        state.clone(),
        paths.staging.clone(),
    ));
    Ok(AppState {
        engine,
        state,
        paths,
    })
}

async fn check_all(engine: &FontEngine) -> Vec<std::result::Result<UpdateStatus, String>> {
    let mut statuses = Vec::new();
    for font in engine.fonts() {
        statuses.push(
            engine
                .check_font(&font.id)
                .await
                .map_err(|error| format!("{}: {error}", font.id)),
        );
    }
    statuses
}

#[tauri::command]
async fn dashboard(state: State<'_, AppState>) -> std::result::Result<Dashboard, String> {
    let installed = state
        .state
        .list_installed()
        .await
        .map_err(|error| error.to_string())?;
    let statuses = check_all(&state.engine)
        .await
        .into_iter()
        .filter_map(std::result::Result::ok)
        .collect();
    let activities = state
        .state
        .list_activity(100)
        .map_err(|error| error.to_string())?;
    Ok(Dashboard {
        fonts: state.engine.fonts(),
        installed,
        statuses,
        activities,
    })
}

#[tauri::command]
async fn check_font(
    font_id: String,
    state: State<'_, AppState>,
) -> std::result::Result<UpdateStatus, String> {
    state
        .engine
        .check_font(&font_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn install_font(
    input: InstallInput,
    state: State<'_, AppState>,
) -> std::result::Result<InstalledFont, String> {
    state
        .engine
        .install(InstallRequest {
            font_id: input.font_id,
            version: input.version,
            variant_ids: input.variant_ids,
            accept_license: input.accept_license,
        })
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn uninstall_font(
    font_id: String,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    state
        .engine
        .uninstall(&font_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn rollback_font(
    font_id: String,
    state: State<'_, AppState>,
) -> std::result::Result<InstalledFont, String> {
    state
        .engine
        .rollback(&font_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn save_source(
    definition: FontDefinition,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    state
        .state
        .save_user_source(&definition)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn set_schedule(input: ScheduleInput) -> std::result::Result<String, String> {
    let result = if input.enabled {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        install_daily_schedule(&executable)
    } else {
        remove_daily_schedule()
    };
    result
        .map(|value| format!("{value:?}"))
        .map_err(|error| error.to_string())
}
