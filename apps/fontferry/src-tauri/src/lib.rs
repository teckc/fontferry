mod cli;
mod commands;
use cli::{Cli, run_cli};

use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use fontferry_core::{
    DeliveryPolicy, FontDefinition, FontEngine, InstallRequest, InstalledFont, StateRepository,
    UpdateStatus,
};
use fontferry_platform::{
    AppPaths, CachedReleaseSource, CatalogVerifier, HttpClient, PlatformFontInstaller,
    SafeFontPreparer, SqliteState, load_embedded_or_cached, refresh_signed_catalog,
    saved_schedule_state, scan_font_awesome, update_daily_schedule,
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
const FONT_AWESOME_PRO: &str = "font-awesome-pro";

pub struct AppState {
    engine: Arc<FontEngine>,
    state: Arc<SqliteState>,
    http: Arc<HttpClient>,
    paths: AppPaths,
    font_scan_started: AtomicBool,
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
    operation_busy: bool,
    schedule_enabled: Option<bool>,
    update_channel: String,
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
            commands::dashboard,
            refresh_local_font_observation,
            commands::check_font,
            commands::check_updates,
            commands::install_font,
            commands::uninstall_font,
            commands::rollback_font,
            commands::save_source,
            commands::set_schedule,
            commands::set_manual_version,
            commands::set_update_channel,
            commands::check_app_update,
            commands::install_app_update,
            commands::refresh_catalog
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
    for source in state.list_user_sources()? {
        if fonts.iter().any(|font| font.id == source.id) {
            anyhow::bail!(
                "user source {} conflicts with catalog; rename the user source before starting",
                source.id
            );
        }
        fonts.push(source);
    }
    let http = Arc::new(HttpClient::new()?);
    let releases = Arc::new(CachedReleaseSource::new((*http).clone(), state.clone()));
    let engine = Arc::new(
        FontEngine::new(
            fonts,
            releases,
            http.clone(),
            Arc::new(SafeFontPreparer),
            Arc::new(PlatformFontInstaller::new(paths.clone())),
            state.clone(),
            paths.staging.clone(),
        )
        .with_operation_lock(paths.data.join("operations.lock")),
    );
    Ok(AppState {
        engine,
        state,
        http,
        paths,
        font_scan_started: AtomicBool::new(false),
        _log_guard: log_guard,
    })
}

fn refresh_font_awesome_scan(state: &SqliteState) -> Result<bool> {
    let Some(observed) = scan_font_awesome() else {
        return Ok(false);
    };
    state.save_scan_result(observed)?;
    Ok(true)
}

/// Called once after the window has rendered cached state. No remote version
/// checks or changes to font ownership take place here.
#[tauri::command]
async fn refresh_local_font_observation(
    state: State<'_, AppState>,
) -> std::result::Result<Option<UpdateStatus>, String> {
    if state.font_scan_started.swap(true, Ordering::Relaxed) {
        return Ok(None);
    }
    let repository = state.state.clone();
    let found =
        tauri::async_runtime::spawn_blocking(move || refresh_font_awesome_scan(&repository))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
    if !found {
        return Ok(None);
    }
    commands::cached_status_for_font(&state, FONT_AWESOME_PRO)
        .await
        .map_err(|error| error.to_string())
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
