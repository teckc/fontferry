<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import type {
    Activity,
    Dashboard,
    FontDefinition,
    InstalledFont,
    UpdateStatus,
  } from "./types";

  type Page = "dashboard" | "catalog" | "sources" | "activity" | "settings";

  let page: Page = "dashboard";
  let data: Dashboard = { fonts: [], installed: [], statuses: [], activities: [] };
  let selected: FontDefinition | null = null;
  let loading = true;
  let checking = false;
  let message = "";
  let error = "";
  let query = "";
  let policy = "all";
  let selectedVariants: string[] = [];
  let scheduleEnabled = true;
  let sourceKind = "github";
  let sourceId = "";
  let sourceName = "";
  let sourceRepository = "";
  let sourceAssetPattern = ".*\\.(zip|7z|tar\\.gz)$";
  let sourceHomepage = "";
  let manualVersion = "";
  let updateChannel = "stable";
  let appUpdateAvailable = false;

  const navigation: { id: Page; label: string; symbol: string }[] = [
    { id: "dashboard", label: "仪表盘", symbol: "◫" },
    { id: "catalog", label: "字体目录", symbol: "Aa" },
    { id: "sources", label: "新增来源", symbol: "+" },
    { id: "activity", label: "活动中心", symbol: "↻" },
    { id: "settings", label: "设置", symbol: "⚙" },
  ];

  onMount(load);

  async function load() {
    loading = true;
    error = "";
    try {
      data = await invoke<Dashboard>("dashboard");
    } catch (cause) {
      error = String(cause);
    } finally {
      loading = false;
    }
  }

  function installed(fontId: string): InstalledFont | undefined {
    return data.installed.find((item) => item.fontId === fontId);
  }

  function status(fontId: string): UpdateStatus | undefined {
    return data.statuses.find((item) => item.fontId === fontId);
  }

  function openFont(font: FontDefinition) {
    selected = font;
    selectedVariants = font.variants.filter((variant) => variant.default).map((variant) => variant.id);
    message = "";
    error = "";
    manualVersion = status(font.id)?.currentVersion ?? "";
  }

  function toggleVariant(id: string) {
    selectedVariants = selectedVariants.includes(id)
      ? selectedVariants.filter((item) => item !== id)
      : [...selectedVariants, id];
  }

  async function check(fontId: string) {
    checking = true;
    error = "";
    try {
      const result = await invoke<UpdateStatus>("check_font", { fontId });
      data.statuses = [...data.statuses.filter((item) => item.fontId !== fontId), result];
      message = result.updateAvailable ? `发现 ${result.availableVersion}` : "已经是最新版本";
    } catch (cause) {
      error = String(cause);
    } finally {
      checking = false;
    }
  }

  async function install(font: FontDefinition) {
    error = "";
    message = "正在下载并验证…";
    try {
      await invoke("install_font", {
        input: {
          fontId: font.id,
          version: status(font.id)?.availableVersion ?? null,
          variantIds: selectedVariants,
          acceptLicense: true,
        },
      });
      message = "安装完成";
      await load();
    } catch (cause) {
      error = String(cause);
      message = "";
    }
  }

  async function remove(font: FontDefinition) {
    if (!window.confirm(`卸载 FontFerry 管理的 ${font.name} 文件？`)) return;
    try {
      await invoke("uninstall_font", { fontId: font.id });
      selected = null;
      await load();
    } catch (cause) {
      error = String(cause);
    }
  }

  async function rollback(font: FontDefinition) {
    try {
      await invoke("rollback_font", { fontId: font.id });
      message = "已恢复上一版本";
      await load();
    } catch (cause) {
      error = String(cause);
    }
  }

  async function saveSource() {
    error = "";
    const repository = sourceRepository.trim();
    const homepage = sourceHomepage.trim() || `https://github.com/${repository}`;
    const notifyOnly = sourceKind === "metadata";
    const definition: FontDefinition = {
      id: sourceId.trim(),
      name: sourceName.trim(),
      description: "用户添加的本地未签名来源",
      homepage,
      license: {
        name: "用户确认的上游许可证",
        url: homepage,
        spdx: null,
        revision: "user-source-v1",
        requiresAcceptance: true,
        redistributionAllowed: false,
      },
      versionProvider:
        sourceKind === "github"
          ? { type: "githubRelease", repository, channel: "stable" }
          : { type: "httpFingerprint", url: homepage },
      artifactProvider:
        sourceKind === "github" ? { type: "githubAsset", repository } : null,
      deliveryPolicy: notifyOnly ? "notifyOnly" : "autoInstall",
      versionPolicy: {},
      variants: notifyOnly
        ? []
        : [
            {
              id: "default",
              name: "默认",
              description: "匹配一个 Release 资产",
              assetPattern: sourceAssetPattern,
              default: true,
            },
          ],
      platforms: ["windows", "macos", "linux"],
    };
    try {
      await invoke("save_source", { definition });
      message = "来源已保存，重启 FontFerry 后载入";
    } catch (cause) {
      error = String(cause);
    }
  }

  async function setSchedule() {
    try {
      message = await invoke<string>("set_schedule", {
        input: { enabled: scheduleEnabled },
      });
    } catch (cause) {
      error = String(cause);
    }
  }

  async function saveManualVersion(font: FontDefinition) {
    try {
      await invoke("set_manual_version", {
        fontId: font.id,
        version: manualVersion || null,
      });
      message = "本地版本修正已保存";
      await check(font.id);
    } catch (cause) {
      error = String(cause);
    }
  }

  async function checkAppUpdate() {
    try {
      const update = await invoke<{ available: boolean; version: string | null }>(
        "check_app_update",
        { channel: updateChannel },
      );
      message = update.available
        ? `FontFerry ${update.version} 可用`
        : "FontFerry 已是当前通道的最新版本";
      appUpdateAvailable = update.available;
    } catch (cause) {
      error = String(cause);
    }
  }

  async function installAppUpdate() {
    try {
      message = "正在下载并验证程序更新…";
      const installed = await invoke<boolean>("install_app_update", {
        channel: updateChannel,
      });
      message = installed
        ? "更新安装已启动；请按系统提示完成或重新启动 FontFerry"
        : "没有可安装的程序更新";
    } catch (cause) {
      error = String(cause);
      message = "";
    }
  }

  async function refreshCatalog() {
    try {
      message = await invoke<string>("refresh_catalog");
    } catch (cause) {
      error = String(cause);
    }
  }

  $: filteredFonts = data.fonts.filter((font) => {
    const text = `${font.name} ${font.id} ${font.description}`.toLowerCase();
    return (
      text.includes(query.toLowerCase()) &&
      (policy === "all" || font.deliveryPolicy === policy)
    );
  });
  $: updateCount = data.statuses.filter((item) => item.updateAvailable).length;
  $: autoCount = data.statuses.filter(
    (item) => item.updateAvailable && item.deliveryPolicy === "autoInstall",
  ).length;
  $: reminderCount = data.statuses.filter(
    (item) => item.updateAvailable && item.deliveryPolicy === "notifyOnly",
  ).length;
</script>

<svelte:head>
  <title>FontFerry</title>
</svelte:head>

<div class="shell">
  <aside>
    <div class="brand">
      <div class="brandmark">F</div>
      <div><strong>FontFerry</strong><small>字体渡口</small></div>
    </div>
    <nav aria-label="主导航">
      {#each navigation as item}
        <button class:active={page === item.id} onclick={() => (page = item.id)}>
          <span>{item.symbol}</span>{item.label}
          {#if item.id === "activity" && updateCount}<b>{updateCount}</b>{/if}
        </button>
      {/each}
    </nav>
    <div class="trust">
      <span class="dot"></span>
      <div><strong>内置目录</strong><small>本地签名基线</small></div>
    </div>
  </aside>

  <main>
    <header>
      <div>
        <p class="eyebrow">FONT OPERATIONS</p>
        <h1>{navigation.find((item) => item.id === page)?.label}</h1>
      </div>
      <button class="quiet" onclick={load} disabled={loading}>↻ 刷新</button>
    </header>

    {#if error}
      <div class="notice error"><span>!</span><div><strong>操作失败</strong><p>{error}</p></div></div>
    {/if}
    {#if message}
      <div class="notice success"><span>✓</span><p>{message}</p></div>
    {/if}

    {#if loading}
      <section class="loading"><div></div><p>正在读取字体状态…</p></section>
    {:else if page === "dashboard"}
      <section class="metrics">
        <article><small>已管理字体</small><strong>{data.installed.length}</strong><p>仅包含 FontFerry 所有文件</p></article>
        <article class="accent"><small>可自动更新</small><strong>{autoCount}</strong><p>许可证已确认的公开制品</p></article>
        <article><small>仅提醒</small><strong>{reminderCount}</strong><p>商业或无公开下载渠道</p></article>
        <article><small>目录字体</small><strong>{data.fonts.length}</strong><p>内置与本地来源</p></article>
      </section>
      <section class="panel">
        <div class="panel-title"><div><h2>需要关注</h2><p>可更新、仅提醒和最近失败集中显示</p></div></div>
        <div class="rows">
          {#each data.statuses.filter((item) => item.updateAvailable) as item}
            {@const font = data.fonts.find((candidate) => candidate.id === item.fontId)}
            {#if font}
              <button class="font-row" onclick={() => openFont(font)}>
                <span class="font-avatar">{font.name.slice(0, 1)}</span>
                <span class="grow"><strong>{font.name}</strong><small>{item.currentVersion ?? "未纳管"} → {item.availableVersion}</small></span>
                <span class:reminder={item.deliveryPolicy === "notifyOnly"} class="pill">
                  {item.deliveryPolicy === "autoInstall" ? "可更新" : "仅提醒"}
                </span>
                <span>›</span>
              </button>
            {/if}
          {:else}
            <div class="empty">暂无已知更新。点击“刷新”重新读取本地状态。</div>
          {/each}
        </div>
      </section>
    {:else if page === "catalog"}
      <section class="toolbar">
        <input aria-label="搜索字体" bind:value={query} placeholder="搜索字体、ID 或描述…" />
        <select aria-label="更新方式" bind:value={policy}>
          <option value="all">全部交付方式</option>
          <option value="autoInstall">可自动安装</option>
          <option value="notifyOnly">仅提醒</option>
        </select>
      </section>
      <section class="catalog-grid">
        {#each filteredFonts as font}
          {@const current = installed(font.id)}
          {@const update = status(font.id)}
          <button class="font-card" onclick={() => openFont(font)}>
            <div class="card-top">
              <span class="font-avatar large">{font.name.slice(0, 1)}</span>
              <span class:reminder={font.deliveryPolicy === "notifyOnly"} class="pill">
                {font.deliveryPolicy === "autoInstall" ? "自动安装" : "仅提醒"}
              </span>
            </div>
            <h2>{font.name}</h2>
            <code>{font.id}</code>
            <p>{font.description}</p>
            <footer>
              <span>{current ? `已安装 ${current.version}` : "未由 FontFerry 管理"}</span>
              {#if update?.updateAvailable}<strong>有更新</strong>{/if}
            </footer>
          </button>
        {/each}
      </section>
    {:else if page === "sources"}
      <section class="split">
        <article class="panel form">
          <div class="panel-title"><div><h2>新增本地来源</h2><p>来源只保存在本机 SQLite，明确标记为未签名</p></div><span class="pill reminder">未签名</span></div>
          <label>来源类型
            <select bind:value={sourceKind}>
              <option value="github">GitHub Release + 资产</option>
              <option value="metadata">网页 HTTP 指纹（仅提醒）</option>
            </select>
          </label>
          <div class="field-pair">
            <label>ID<input bind:value={sourceId} placeholder="my-font" /></label>
            <label>显示名称<input bind:value={sourceName} placeholder="My Font" /></label>
          </div>
          {#if sourceKind === "github"}
            <label>GitHub 仓库<input bind:value={sourceRepository} placeholder="owner/repository" /></label>
            <label>资产匹配正则<input bind:value={sourceAssetPattern} /></label>
          {:else}
            <label>公开 HTTPS 地址<input bind:value={sourceHomepage} placeholder="https://example.com/font/" /></label>
          {/if}
          <button class="primary" onclick={saveSource} disabled={!sourceId || !sourceName}>验证并保存</button>
        </article>
        <article class="panel guidance">
          <h2>安全边界</h2>
          <ul>
            <li>只接受声明式版本与资产规则，不执行任意脚本。</li>
            <li>下载必须使用公开 HTTPS，拒绝本机和私有网络地址。</li>
            <li>一个变体必须恰好匹配一个 Release 资产。</li>
            <li>压缩包受路径、符号链接、体积和条目数限制。</li>
          </ul>
          <p>保存前程序会验证 schema 和正则；正式目录由独立 Ed25519 密钥签名。</p>
        </article>
      </section>
    {:else if page === "activity"}
      <section class="panel">
        <div class="panel-title"><div><h2>操作与通知</h2><p>本地日志不包含 URL 查询参数和敏感路径</p></div></div>
        <div class="timeline">
          {#each data.activities as item}
            <div class="event">
              <span class:bad={item.level === "error"} class:warn={item.level === "warning"}></span>
              <div><strong>{item.fontId ?? "FontFerry"}</strong><p>{item.message}</p></div>
              <time>{new Date(item.createdAt).toLocaleString()}</time>
            </div>
          {:else}
            <div class="empty">尚无活动记录。</div>
          {/each}
        </div>
      </section>
    {:else if page === "settings"}
      <section class="settings">
        <article class="panel">
          <h2>每日检查</h2>
          <label class="switch-row"><span><strong>启用系统计划任务</strong><small>每天检查；Linux 无 systemd 时在应用启动补检</small></span><input type="checkbox" bind:checked={scheduleEnabled} /></label>
          <button class="primary" onclick={setSchedule}>应用计划</button>
        </article>
        <article class="panel">
          <h2>程序更新</h2>
          <div class="setting-row"><span><strong>更新通道</strong><small>稳定版使用 latest.json，测试版使用 beta 清单</small></span><select bind:value={updateChannel}><option value="stable">稳定</option><option value="beta">测试</option></select></div>
          <button class="primary" onclick={checkAppUpdate}>检查程序更新</button>
          {#if appUpdateAvailable}
            <button class="quiet" onclick={installAppUpdate}>下载并安装已签名更新</button>
          {/if}
          <p class="muted">AppImage 支持自动更新；deb/rpm 仅提醒并交给系统包管理器。</p>
        </article>
        <article class="panel">
          <h2>目录信任</h2>
          <div class="setting-row"><span><strong>内置目录</strong><small>builtin-2026-07-29</small></span><span class="pill">可用</span></div>
          <div class="setting-row"><span><strong>远程签名目录</strong><small>验签失败时使用最后成功缓存</small></span><span class="pill reminder">待配置密钥</span></div>
          <button class="quiet" onclick={refreshCatalog}>刷新并验签目录</button>
        </article>
      </section>
    {/if}
  </main>
</div>

{#if selected}
  <div class="backdrop" role="presentation" onclick={(event) => event.target === event.currentTarget && (selected = null)}>
    <div class="drawer" role="dialog" aria-modal="true" aria-label={selected.name}>
      <button class="close" aria-label="关闭" onclick={() => (selected = null)}>×</button>
      <p class="eyebrow">{selected.id}</p>
      <h1>{selected.name}</h1>
      <p class="lead">{selected.description}</p>
      <div class="detail-grid">
        <div><small>当前版本</small><strong>{installed(selected.id)?.version ?? "未纳管"}</strong></div>
        <div><small>最新版本</small><strong>{status(selected.id)?.availableVersion ?? "尚未检查"}</strong></div>
        <div><small>交付策略</small><strong>{selected.deliveryPolicy === "autoInstall" ? "可自动安装" : "仅提醒"}</strong></div>
        <div><small>许可证</small><strong>{selected.license.spdx ?? "商业/自定义"}</strong></div>
      </div>
      {#if selected.variants.length}
        <h3>选择变体</h3>
        <div class="variants">
          {#each selected.variants as variant}
            <label class:selected={selectedVariants.includes(variant.id)}>
              <input type="checkbox" checked={selectedVariants.includes(variant.id)} onchange={() => toggleVariant(variant.id)} />
              <span><strong>{variant.name}</strong><small>{variant.description}</small></span>
            </label>
          {/each}
        </div>
      {/if}
      <div class="license-line"><span>许可证修订：{selected.license.revision}</span><a href={selected.license.url} target="_blank" rel="noreferrer">查看许可证 ↗</a></div>
      {#if selected.deliveryPolicy === "notifyOnly"}
        <div class="manual-version">
          <label>本地版本修正<input bind:value={manualVersion} placeholder="例如 7.2.0" /></label>
          <button class="quiet" onclick={() => saveManualVersion(selected!)}>保存</button>
        </div>
      {/if}
      <div class="drawer-actions">
        <button class="quiet" onclick={() => check(selected!.id)} disabled={checking}>{checking ? "检查中…" : "检查更新"}</button>
        {#if selected.deliveryPolicy === "autoInstall"}
          <button class="primary" onclick={() => install(selected!)} disabled={!selectedVariants.length}>安装 / 更新</button>
        {:else}
          <a class="primary link" href={selected.homepage} target="_blank" rel="noreferrer">前往官方渠道</a>
        {/if}
      </div>
      {#if installed(selected.id)}
        <div class="danger-zone">
          {#if installed(selected.id)?.previous}<button onclick={() => rollback(selected!)}>回滚到上一版本</button>{/if}
          <button onclick={() => remove(selected!)}>卸载已管理文件</button>
        </div>
      {/if}
    </div>
  </div>
{/if}
