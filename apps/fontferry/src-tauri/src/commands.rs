use super::*;

#[tauri::command]
pub(super) async fn dashboard(
    state: State<'_, AppState>,
) -> std::result::Result<Dashboard, String> {
    state
        .engine
        .recover()
        .await
        .map_err(|error| error.to_string())?;
    let _guard = state
        .engine
        .acquire_operation_lock()
        .map_err(|e| e.to_string())?;
    let installed = state
        .state
        .list_installed()
        .await
        .map_err(|e| e.to_string())?;
    let statuses = cached_statuses(&state, &installed)
        .await
        .map_err(|e| e.to_string())?;
    let activities = state
        .state
        .list_activity(100)
        .map_err(|error| error.to_string())?;
    Ok(Dashboard {
        fonts: state.engine.fonts(),
        installed,
        statuses,
        activities,
        schedule_enabled: saved_schedule_state(&state.state).map_err(|e| e.to_string())?,
        update_channel: state
            .state
            .get_setting::<String>("update-channel")
            .map_err(|e| e.to_string())?
            .filter(|channel| matches!(channel.as_str(), "stable" | "beta"))
            .unwrap_or_else(|| "stable".into()),
        github_proxy_port: state
            .state
            .get_setting::<Option<NonZeroU16>>(GITHUB_PROXY_PORT_SETTING)
            .map_err(|e| e.to_string())?
            .flatten(),
    })
}

#[tauri::command]
pub(super) async fn check_font(
    font_id: String,
    state: State<'_, AppState>,
) -> std::result::Result<UpdateStatus, InstallFailure> {
    let status = state.engine.check_font(&font_id).await?;
    cache_status(&state.state, &status).map_err(|error| InstallFailure {
        kind: "other",
        message: error.to_string(),
    })?;
    Ok(status)
}

#[tauri::command]
pub(super) async fn check_updates(
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
    let mut failures = Vec::new();
    for (font_id, result) in font_ids.iter().zip(results) {
        match result {
            Ok(status) => {
                cache_status(&state.state, &status).map_err(|error| error.to_string())?;
                statuses.push(status);
            }
            Err(error) => {
                failures.push(font_id.clone());
                state
                    .engine
                    .record_failure(font_id, &error.to_string())
                    .await
                    .map_err(|e| e.to_string())?;
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

async fn cached_statuses(
    state: &AppState,
    installed: &[InstalledFont],
) -> Result<Vec<UpdateStatus>> {
    let mut statuses = Vec::new();
    for font in state.engine.fonts() {
        let Some(mut status) = state
            .state
            .get_setting::<UpdateStatus>(&status_cache_key(&font.id))?
        else {
            continue;
        };
        let observed = state.state.get_observed(&font.id).await?;
        let current = installed
            .iter()
            .find(|i| i.font_id == font.id)
            .map(|i| {
                i.manual_version
                    .clone()
                    .unwrap_or_else(|| i.version.clone())
            })
            .or_else(|| observed.and_then(|i| i.manual_version.or(i.detected_version)));
        status.refresh_local(
            current,
            matches!(
                font.version_provider,
                fontferry_core::VersionProvider::HttpFingerprint { .. }
            ),
        );
        statuses.push(status);
    }
    Ok(statuses)
}

#[tauri::command]
pub(super) async fn prepare_font_install(
    font_id: String,
    version: Option<String>,
    variant_ids: Vec<String>,
    state: State<'_, AppState>,
) -> std::result::Result<InstallCandidate, InstallFailure> {
    state
        .engine
        .prepare_install(&font_id, version.as_deref(), &variant_ids)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub(super) async fn install_font(
    input: InstallInput,
    progress: tauri::ipc::Channel<InstallStage>,
    state: State<'_, AppState>,
) -> std::result::Result<InstallReport, InstallFailure> {
    state
        .engine
        .install_with_progress(
            InstallRequest {
                font_id: input.font_id,
                version: input.version,
                variant_ids: input.variant_ids,
                accept_license: input.accept_license,
            },
            Some(&input.candidate),
            |stage| {
                let _delivery = progress.send(stage);
            },
        )
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub(super) async fn uninstall_font(
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
pub(super) async fn rollback_font(
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
pub(super) async fn save_source(
    definition: FontDefinition,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let user_ids: Vec<_> = state
        .state
        .list_user_sources()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|f| f.id)
        .collect();
    if state.engine.fonts().iter().any(|f| f.id == definition.id)
        && !user_ids.contains(&definition.id)
    {
        return Err("自定义字体 ID 与内置目录冲突，请使用不同 ID".into());
    }
    state
        .state
        .save_user_source(&definition)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn set_schedule(
    input: ScheduleInput,
    state: State<'_, AppState>,
) -> std::result::Result<String, String> {
    let engine = state.engine.clone();
    let repository = state.state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        update_daily_schedule(&engine, &repository, &executable, input.enabled)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(if result.enabled {
        "每日检查已启用"
    } else {
        "每日检查已禁用"
    }
    .into())
}

#[tauri::command]
pub(super) async fn set_manual_version(
    font_id: String,
    version: Option<String>,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    let _guard = state
        .engine
        .acquire_operation_lock()
        .map_err(|e| e.to_string())?;
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
pub(super) async fn set_update_channel(
    channel: String,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    update_endpoint(&channel)?;
    state
        .state
        .set_setting("update-channel", &channel)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn set_github_proxy_port(
    port: Option<NonZeroU16>,
    state: State<'_, AppState>,
) -> std::result::Result<(), String> {
    state
        .state
        .set_setting(GITHUB_PROXY_PORT_SETTING, &port)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn probe_github_network(
    port: Option<NonZeroU16>,
) -> std::result::Result<NetworkProbe, String> {
    let client = HttpClient::with_github_proxy(port).map_err(|error| error.to_string())?;
    let (releases, download) = client.probe_github().await;
    Ok(NetworkProbe {
        releases_error: releases.err().map(|error| error.to_string()),
        download_error: download.err().map(|error| error.to_string()),
    })
}

#[tauri::command]
pub(super) async fn check_app_update(
    channel: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> std::result::Result<AppUpdate, String> {
    let endpoint = update_endpoint(&channel)?;
    state
        .state
        .set_setting("update-channel", &channel)
        .map_err(|error| error.to_string())?;
    let updater = app_updater(&app, endpoint, state.github_proxy_url.clone())?;
    let update = updater.check().await.map_err(app_update_error)?;
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
pub(super) async fn install_app_update(
    channel: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> std::result::Result<bool, String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_none() {
        return Err("deb/rpm 安装由 apt 或 dnf 管理，FontFerry 不会覆盖包管理器文件".into());
    }

    let endpoint = update_endpoint(&channel)?;
    let updater = app_updater(&app, endpoint, state.github_proxy_url.clone())?;
    let Some(update) = updater.check().await.map_err(app_update_error)? else {
        return Ok(false);
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;
    Ok(true)
}

fn app_update_error(error: tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error::*;
    match error {
        ReleaseNotFound | Serialization(_) => "更新通道暂无可用清单，请前往发行页获取安装包".into(),
        TargetNotFound(_) | TargetsNotFound(_) => {
            "此更新通道暂无适用于本机的安装包，请查看发行页".into()
        }
        _ => error.to_string(),
    }
}

fn app_updater(
    app: &tauri::AppHandle,
    endpoint: Url,
    proxy: Option<Url>,
) -> std::result::Result<tauri_plugin_updater::Updater, String> {
    let mut builder = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|error| error.to_string())?;
    if let Some(proxy) = proxy {
        builder = builder.proxy(proxy);
    }
    builder.build().map_err(|error| error.to_string())
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
pub(super) async fn refresh_catalog(
    state: State<'_, AppState>,
) -> std::result::Result<String, String> {
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
