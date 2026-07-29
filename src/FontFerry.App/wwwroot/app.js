const state = {
  catalog: [],
  installed: {},
  system: null,
  activeFont: null,
  releases: []
};

const $ = selector => document.querySelector(selector);

async function api(url, options = {}) {
  const headers = { ...(options.headers || {}) };
  if (options.body) headers["Content-Type"] = "application/json";
  if ((options.method || "GET") !== "GET") headers["X-FontFerry-Request"] = "1";
  const response = await fetch(url, { ...options, headers });
  if (!response.ok) {
    const payload = await response.json().catch(() => ({}));
    throw new Error(payload.detail || payload.error || `${response.status} ${response.statusText}`);
  }
  if (response.status === 204) return null;
  return response.json();
}

async function bootstrap() {
  try {
    const [system, catalog, installState, schedule] = await Promise.all([
      api("/api/system"),
      api("/api/catalog"),
      api("/api/state"),
      api("/api/schedule")
    ]);
    state.system = system;
    state.catalog = catalog;
    state.installed = installState.installed || {};
    $("#schedule-toggle").checked = schedule.enabled;
    $("#platform-badge").textContent =
      `${system.platform === "macos" ? "macOS" : "Windows"} · 本地连接`;
    renderCatalog();
  } catch (error) {
    $("#font-grid").innerHTML = `<p class="empty">${escapeHtml(error.message)}</p>`;
  }
}

function renderCatalog() {
  const query = $("#search").value.trim().toLowerCase();
  const fonts = state.catalog.filter(font =>
    `${font.name} ${font.id} ${font.source.repository || ""}`.toLowerCase().includes(query));
  const grid = $("#font-grid");
  grid.replaceChildren();
  $("#catalog-summary").textContent =
    `${state.catalog.length} 个来源 · ${Object.keys(state.installed).length} 个已安装`;

  if (!fonts.length) {
    grid.innerHTML = `<p class="empty">没有匹配的字体。</p>`;
    return;
  }

  const template = $("#font-card-template");
  fonts.forEach(font => {
    const node = template.content.cloneNode(true);
    const installed = state.installed[font.id];
    node.querySelector("h3").textContent = font.name;
    node.querySelector(".description").textContent = font.description || "用户添加的字体发行来源。";
    node.querySelector(".source-kind").textContent =
      font.source.type === "GitHubRelease" ? "GitHub Release" : "Official Website";
    const installState = node.querySelector(".install-state");
    installState.textContent = installed ? `已安装 ${installed.version}` : "未安装";
    installState.classList.toggle("installed", Boolean(installed));
    node.querySelector(".card-meta").textContent =
      `${font.source.channel === "Prerelease" ? "含预发布版" : "稳定通道"} · ${font.license.name}`;
    const homepage = node.querySelector(".homepage");
    homepage.href = font.homepage;
    node.querySelector(".manage-button").addEventListener("click", () => openManage(font));
    grid.appendChild(node);
  });
}

async function openManage(font) {
  state.activeFont = font;
  state.releases = [];
  $("#manage-title").textContent = font.name;
  $("#manage-status").textContent = "正在读取可用版本…";
  $("#manage-content").innerHTML = `<p class="empty">正在连接上游…</p>`;
  $("#install-button").disabled = true;
  $("#manage-dialog").showModal();

  try {
    if (font.source.type === "GitHubRelease") {
      state.releases = await api(`/api/catalog/${encodeURIComponent(font.id)}/releases`);
    }
    renderManage(font);
    $("#manage-status").textContent = "";
    $("#install-button").disabled = false;
  } catch (error) {
    $("#manage-content").innerHTML = `<p class="empty">${escapeHtml(error.message)}</p>`;
    $("#manage-status").textContent = "无法读取发行信息";
  }
}

function renderManage(font) {
  const installed = state.installed[font.id];
  const content = $("#manage-content");
  if (font.source.type === "StaticUrl") {
    content.innerHTML = `
      <div class="field"><span>字体包</span><div class="asset-list" id="asset-list"></div></div>
      ${licenseMarkup(font)}
      ${installed ? dangerMarkup(installed) : ""}`;
    renderStaticPresets(font);
  } else {
    const validReleases = state.releases.filter(release =>
      font.source.channel === "Prerelease" || !release.prerelease);
    if (!validReleases.length) {
      content.innerHTML = `<p class="empty">没有符合更新通道的 Release。</p>`;
      return;
    }
    content.innerHTML = `
      <label class="field"><span>活动版本（仅安装一个版本）</span>
        <select id="release-select">
          ${validReleases.map(release => `<option value="${escapeHtml(release.tag)}">
            ${escapeHtml(release.tag)}${release.prerelease ? " · 预发布" : ""}
          </option>`).join("")}
        </select>
      </label>
      <div class="field"><span>发行资产与变体</span><div class="asset-list" id="asset-list"></div></div>
      ${licenseMarkup(font)}
      ${installed ? dangerMarkup(installed) : ""}`;
    $("#release-select").addEventListener("change", () => renderReleaseAssets(font));
    renderReleaseAssets(font);
  }

  const uninstall = $("#uninstall-button");
  if (uninstall) uninstall.addEventListener("click", () => uninstallFont(font));
  const rollback = $("#rollback-button");
  if (rollback) rollback.addEventListener("click", () => rollbackFont(font));
}

function renderReleaseAssets(font) {
  const tag = $("#release-select").value;
  const release = state.releases.find(item => item.tag === tag);
  const installed = state.installed[font.id];
  const selectedTokens = new Set(installed?.assets || []);
  const options = font.presets.length
    ? font.presets.map(preset => {
        const asset = release.assets.find(item => new RegExp(preset.assetPattern).test(item.name));
        if (!asset) return "";
        const checked = selectedTokens.has(preset.id) || selectedTokens.has(asset.name) ||
          (!installed && preset.default);
        return assetOption(preset.id, preset.name, asset.size, checked, preset.description);
      })
    : release.assets
        .filter(asset => /\.(zip|7z|tar\.gz|tgz|ttf|otf|ttc|otc)$/i.test(asset.name))
        .map(asset => assetOption(
          asset.name, asset.name, asset.size, selectedTokens.has(asset.name)));
  $("#asset-list").innerHTML = options.join("") ||
    `<p class="empty">此版本没有匹配的字体资产。</p>`;
}

function renderStaticPresets(font) {
  const installed = state.installed[font.id];
  const selected = new Set(installed?.assets || []);
  $("#asset-list").innerHTML = font.presets.map(preset =>
    assetOption(preset.id, preset.name, null,
      selected.has(preset.id) || (!installed && preset.default), preset.description)
  ).join("");
}

function assetOption(value, label, size, checked, description = "") {
  return `
    <label class="asset-option">
      <input type="checkbox" name="asset" value="${escapeHtml(value)}" ${checked ? "checked" : ""}>
      <span><strong>${escapeHtml(label)}</strong>
      <small>${description ? escapeHtml(description) : size ? formatBytes(size) : escapeHtml(value)}</small></span>
    </label>`;
}

function licenseMarkup(font) {
  return `
    <div class="license-box">
      <label>
        <input id="license-accept" type="checkbox" ${font.license.requiresAcceptance ? "" : "checked"}>
        <span>我已阅读并接受 <a href="${escapeHtml(font.license.url)}" target="_blank" rel="noreferrer">
          ${escapeHtml(font.license.name)}</a>。字体仍受上游协议约束。</span>
      </label>
    </div>`;
}

function dangerMarkup(installed) {
  return `
    <div class="danger-row">
      <span>当前安装 ${escapeHtml(installed.version)}</span>
      <span>
        ${installed.previousVersion ? `<button id="rollback-button" class="button ghost" type="button">
          回滚至 ${escapeHtml(installed.previousVersion)}</button>` : ""}
        <button id="uninstall-button" class="button danger" type="button">卸载字体</button>
      </span>
    </div>`;
}

async function installActiveFont() {
  const font = state.activeFont;
  const assets = [...document.querySelectorAll('input[name="asset"]:checked')].map(input => input.value);
  const license = $("#license-accept");
  if (!assets.length) return setManageStatus("至少选择一个字体资产。", true);
  if (font.license.requiresAcceptance && !license.checked)
    return setManageStatus("安装前必须接受字体许可证。", true);

  const version = $("#release-select")?.value || null;
  setManageBusy(true, "正在下载、校验并安装；大型 CJK 字体需要一些时间…");
  try {
    const result = await api(`/api/catalog/${encodeURIComponent(font.id)}/install`, {
      method: "POST",
      body: JSON.stringify({ version, assets, acceptLicense: license.checked })
    });
    const latestState = await api("/api/state");
    state.installed = latestState.installed || {};
    renderCatalog();
    setManageStatus(
      `已安装 ${result.version}，共 ${result.fileCount} 个字体文件。` +
      (result.restartRecommended ? " 请重启使用该字体的应用。" : ""));
  } catch (error) {
    setManageStatus(error.message, true);
  } finally {
    setManageBusy(false);
  }
}

async function uninstallFont(font) {
  if (!confirm(`卸载 ${font.name}？本地下载缓存不会被删除。`)) return;
  setManageBusy(true, "正在卸载…");
  try {
    await api(`/api/catalog/${encodeURIComponent(font.id)}/install`, { method: "DELETE" });
    delete state.installed[font.id];
    renderCatalog();
    $("#manage-dialog").close();
  } catch (error) {
    setManageStatus(error.message, true);
  } finally {
    setManageBusy(false);
  }
}

async function rollbackFont(font) {
  if (!confirm(`将 ${font.name} 回滚到上一版本？`)) return;
  setManageBusy(true, "正在恢复上一版本…");
  try {
    const result = await api(`/api/catalog/${encodeURIComponent(font.id)}/rollback`, {
      method: "POST"
    });
    const latestState = await api("/api/state");
    state.installed = latestState.installed || {};
    renderCatalog();
    setManageStatus(`已回滚至 ${result.version}。请重启使用该字体的应用。`);
    renderManage(font);
  } catch (error) {
    setManageStatus(error.message, true);
  } finally {
    setManageBusy(false);
  }
}

async function updateAll() {
  const button = $("#update-all");
  button.disabled = true;
  button.textContent = "正在更新…";
  try {
    const results = await api("/api/update-all", { method: "POST" });
    const failed = results.filter(result => !result.success);
    const latestState = await api("/api/state");
    state.installed = latestState.installed || {};
    renderCatalog();
    alert(failed.length
      ? `${results.length - failed.length} 个更新成功，${failed.length} 个失败。`
      : `已完成 ${results.length} 个字体的更新检查。`);
  } catch (error) {
    alert(error.message);
  } finally {
    button.disabled = false;
    button.textContent = "更新全部";
  }
}

async function addSource(event) {
  event.preventDefault();
  const form = new FormData(event.currentTarget);
  const repository = form.get("repository").trim();
  const payload = {
    repository,
    name: form.get("name").trim(),
    id: form.get("id").trim(),
    channel: form.get("channel"),
    homepage: form.get("homepage").trim(),
    licenseName: form.get("licenseName").trim(),
    licenseUrl: form.get("licenseUrl").trim()
  };
  $("#add-status").textContent = "正在验证仓库…";
  try {
    await api("/api/catalog", { method: "POST", body: JSON.stringify(payload) });
    state.catalog = await api("/api/catalog");
    renderCatalog();
    $("#add-dialog").close();
    event.currentTarget.reset();
  } catch (error) {
    $("#add-status").textContent = error.message;
  }
}

function setManageBusy(busy, message = "") {
  $("#install-button").disabled = busy;
  if (message) $("#manage-status").textContent = message;
}
function setManageStatus(message, error = false) {
  const element = $("#manage-status");
  element.textContent = message;
  element.style.color = error ? "#a52d20" : "";
}
function formatBytes(bytes) {
  if (!bytes) return "";
  const units = ["B", "KB", "MB", "GB"];
  const power = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** power).toFixed(power > 1 ? 1 : 0)} ${units[power]}`;
}
function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>"']/g, character => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"
  })[character]);
}

$("#search").addEventListener("input", renderCatalog);
$("#install-button").addEventListener("click", installActiveFont);
$("#update-all").addEventListener("click", updateAll);
$("#add-source").addEventListener("click", () => $("#add-dialog").showModal());
$("#add-form").addEventListener("submit", addSource);
document.querySelectorAll("[data-close]").forEach(button =>
  button.addEventListener("click", () => $(`#${button.dataset.close}`).close()));
$("#schedule-toggle").addEventListener("change", async event => {
  const enabled = event.target.checked;
  try {
    await api("/api/schedule", {
      method: "POST",
      body: JSON.stringify({ enabled })
    });
  } catch (error) {
    event.target.checked = !enabled;
    alert(error.message);
  }
});

bootstrap();
