use std::{fs, sync::Arc};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use fontferry_core::{
    DeliveryPolicy, FontDefinition, FontEngine, InstallRequest, InstalledFont, StateRepository,
    UpdateStatus,
};
use fontferry_platform::{
    AppPaths, CachedReleaseSource, CatalogVerifier, HttpClient, PlatformFontInstaller,
    SafeFontPreparer, SqliteState, install_daily_schedule, load_embedded_or_cached,
    refresh_signed_catalog, remove_daily_schedule, scan_font_awesome,
};
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use tauri_plugin_updater::UpdaterExt;
use tracing_subscriber::EnvFilter;
use url::Url;

const CATALOG_JSON: &str = include_str!("../../../../catalog/builtin/catalog.json");
const CATALOG_PUBLIC_KEY: &str = include_str!("../../../../catalog/public-key.txt");
const REMOTE_CATALOG: &str =
    "https://raw.githubusercontent.com/teckc/fontferry/catalog/catalog.json";
const REMOTE_CATALOG_SIGNATURE: &str =
    "https://raw.githubusercontent.com/teckc/fontferry/catalog/catalog.json.sig";

#[derive(Debug, Parser)]
#[command(
    name = "fontferry",
    version,
    about = "Cross-platform font update manager"
)]
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
    http: Arc<HttpClient>,
    paths: AppPaths,
    _log_guard: tracing_appender::non_blocking::WorkerGuard,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdate {
    available: bool,
    version: Option<String>,
    notes: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCheckResult {
    statuses: Vec<UpdateStatus>,
    failures: usize,
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
                        continue;
                    }
                };
                if !status.update_available || status.current_version.is_none() {
                    continue;
                }
                if status.delivery_policy == DeliveryPolicy::AutoInstall {
                    let Some(item) = installed_by_id.get(&font.id) else {
                        continue;
                    };
                    let request = InstallRequest {
                        font_id: item.font_id.clone(),
                        version: status.available_version,
                        variant_ids: item.variant_ids.clone(),
                        accept_license: false,
                    };
                    match state.engine.install(request).await {
                        Ok(installed) => println!("{} -> {}", installed.font_id, installed.version),
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
            check_updates,
            install_font,
            uninstall_font,
            rollback_font,
            save_source,
            set_schedule,
            set_manual_version,
            check_app_update,
            install_app_update,
            refresh_catalog
        ])
        .run(tauri::generate_context!())
        .context("run Tauri application")
}

fn create_state() -> Result<AppState> {
    let paths = AppPaths::discover()?;
    let log_guard = init_logging(&paths)?;
    let verifier = CatalogVerifier::from_base64(CATALOG_PUBLIC_KEY).ok();
    let catalog = load_embedded_or_cached(
        CATALOG_JSON.as_bytes(),
        &paths.catalog_cache_body(),
        &paths.catalog_cache_signature(),
        verifier.as_ref(),
    )
    .context("load catalog")?;
    let mut fonts = catalog.fonts;
    let state = Arc::new(SqliteState::open(&paths.database())?);
    if let Some(observed) = scan_font_awesome() {
        state.save_scan_result(observed)?;
    }
    fonts.extend(state.list_user_sources()?);
    let http = Arc::new(HttpClient::new()?);
    let releases = Arc::new(CachedReleaseSource::new((*http).clone(), state.clone()));
    let engine = Arc::new(FontEngine::new(
        fonts,
        releases,
        http.clone(),
        Arc::new(SafeFontPreparer),
        Arc::new(PlatformFontInstaller::new(paths.clone())),
        state.clone(),
        paths.staging.clone(),
    ));
    Ok(AppState {
        engine,
        state,
        http,
        paths,
        _log_guard: log_guard,
    })
}

fn init_logging(paths: &AppPaths) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let appender = tracing_appender::rolling::daily(&paths.logs, "fontferry.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(writer)
        .finish();
    let _already_initialized = tracing::subscriber::set_global_default(subscriber);

    let mut logs: Vec<_> = fs::read_dir(&paths.logs)
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("fontferry.log")
        })
        .collect();
    logs.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    let remove_count = logs.len().saturating_sub(14);
    for entry in logs.into_iter().take(remove_count) {
        let _ignored = fs::remove_file(entry.path());
    }
    Ok(guard)
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
    let statuses = cached_statuses(&state);
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
    let status = state
        .engine
        .check_font(&font_id)
        .await
        .map_err(|error| error.to_string())?;
    cache_status(&state.state, &status).map_err(|error| error.to_string())?;
    Ok(status)
}

#[tauri::command]
async fn check_updates(
    state: State<'_, AppState>,
) -> std::result::Result<UpdateCheckResult, String> {
    let font_ids = state
        .engine
        .fonts()
        .into_iter()
        .map(|font| font.id)
        .collect::<Vec<_>>();
    let checks = font_ids
        .iter()
        .map(|font_id| state.engine.check_font(font_id));
    let results = join_all(checks).await;
    let mut statuses = Vec::new();
    let mut failures = 0;
    for result in results {
        match result {
            Ok(status) => {
                cache_status(&state.state, &status).map_err(|error| error.to_string())?;
                statuses.push(status);
            }
            Err(error) => {
                failures += 1;
                tracing::warn!(error = %error, "font update check failed");
            }
        }
    }
    Ok(UpdateCheckResult { statuses, failures })
}

fn status_cache_key(font_id: &str) -> String {
    format!("update-status:{font_id}")
}

fn cache_status(state: &SqliteState, status: &UpdateStatus) -> Result<()> {
    state.set_setting(&status_cache_key(&status.font_id), status)?;
    Ok(())
}

fn cached_statuses(state: &AppState) -> Vec<UpdateStatus> {
    state
        .engine
        .fonts()
        .into_iter()
        .filter_map(|font| {
            state
                .state
                .get_setting::<UpdateStatus>(&status_cache_key(&font.id))
                .ok()
                .flatten()
        })
        .collect()
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

#[tauri::command]
async fn set_manual_version(
    font_id: String,
    version: Option<String>,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let normalized = version.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    });
    state
        .state
        .set_observed_manual_version(&font_id, normalized)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn check_app_update(
    channel: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> std::result::Result<AppUpdate, String> {
    state
        .state
        .set_setting("update-channel", &channel)
        .map_err(|error| error.to_string())?;
    let endpoint = update_endpoint(&channel)?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())?;
    let update = updater.check().await.map_err(|error| error.to_string())?;
    Ok(match update {
        Some(update) => AppUpdate {
            available: true,
            version: Some(update.version),
            notes: update.body,
        },
        None => AppUpdate {
            available: false,
            version: None,
            notes: None,
        },
    })
}

#[tauri::command]
async fn install_app_update(
    channel: String,
    app: tauri::AppHandle,
) -> std::result::Result<bool, String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_none() {
        return Err("deb/rpm 安装由 apt 或 dnf 管理，FontFerry 不会覆盖包管理器文件".into());
    }

    let endpoint = update_endpoint(&channel)?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())?;
    let Some(update) = updater.check().await.map_err(|error| error.to_string())? else {
        return Ok(false);
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;
    Ok(true)
}

fn update_endpoint(channel: &str) -> std::result::Result<Url, String> {
    let endpoint = match channel {
        "stable" => "https://github.com/teckc/fontferry/releases/latest/download/latest.json",
        "beta" => "https://github.com/teckc/fontferry/releases/download/beta/latest.json",
        _ => return Err("unknown update channel".into()),
    };
    Url::parse(endpoint).map_err(|error| error.to_string())
}

#[tauri::command]
async fn refresh_catalog(state: State<'_, AppState>) -> std::result::Result<String, String> {
    let verifier = CatalogVerifier::from_base64(CATALOG_PUBLIC_KEY)
        .map_err(|_| "catalog public key is not configured".to_owned())?;
    let catalog_url = Url::parse(REMOTE_CATALOG).map_err(|error| error.to_string())?;
    let signature_url = Url::parse(REMOTE_CATALOG_SIGNATURE).map_err(|error| error.to_string())?;
    let catalog = refresh_signed_catalog(
        &state.http,
        &catalog_url,
        &signature_url,
        &state.paths.catalog_cache_body(),
        &state.paths.catalog_cache_signature(),
        &verifier,
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(format!(
        "目录 {} 已验证，重启后载入 {} 个条目",
        catalog.revision,
        catalog.fonts.len()
    ))
}
