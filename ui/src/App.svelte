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
  type Theme = "system" | "light" | "dark";

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
  let theme: Theme = "system";

  const navigation: { id: Page; label: string; symbol: string }[] = [
    { id: "dashboard", label: "概览", symbol: "◫" },
    { id: "catalog", label: "字体", symbol: "Aa" },
    { id: "sources", label: "添加字体", symbol: "+" },
    { id: "activity", label: "记录", symbol: "↻" },
    { id: "settings", label: "设置", symbol: "⚙" },
  ];

  onMount(() => {
    const savedTheme = localStorage.getItem("fontferry-theme");
    if (savedTheme === "light" || savedTheme === "dark") theme = savedTheme;
    applyTheme();
    void load();
  });

  function applyTheme() {
    if (theme === "system") {
      document.documentElement.removeAttribute("data-theme");
      localStorage.removeItem("fontferry-theme");
    } else {
      document.documentElement.dataset.theme = theme;
      localStorage.setItem("fontferry-theme", theme);
    }
  }

  function changeTheme() {
    applyTheme();
  }

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

  async function checkAllUpdates() {
    checking = true;
    error = "";
    message = "正在检查字体更新…";
    try {
      const result = await invoke<{ statuses: UpdateStatus[]; failures: number }>(
        "check_updates",
      );
      const refreshedIds = new Set(result.statuses.map((item) => item.fontId));
      data.statuses = [
        ...data.statuses.filter((item) => !refreshedIds.has(item.fontId)),
        ...result.statuses,
      ];
      message = result.failures
        ? `检查完成；${result.failures} 个来源暂时无法连接，已保留原有结果`
        : "字体更新检查完成";
    } catch (cause) {
      error = String(cause);
      message = "";
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
    if (!window.confirm(`卸载由字渡安装的 ${font.name}？`)) return;
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
      description: "用户添加的字体",
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
              name: "默认文件",
              description: "使用符合下载文件匹配规则的文件",
              assetPattern: sourceAssetPattern,
              default: true,
            },
          ],
      platforms: ["windows", "macos", "linux"],
    };
    try {
      await invoke("save_source", { definition });
      message = "字体已保存，重启字渡后即可看到";
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
      message = "已保存当前版本";
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
        ? `字渡 ${update.version} 可用`
        : "字渡已是当前更新通道的最新版本";
      appUpdateAvailable = update.available;
    } catch (cause) {
      error = String(cause);
    }
  }

  async function installAppUpdate() {
    try {
      message = "正在下载程序更新…";
      const installed = await invoke<boolean>("install_app_update", {
        channel: updateChannel,
      });
      message = installed
        ? "更新已开始安装；请按系统提示完成或重新启动字渡"
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
      <div class="brandmark" aria-hidden="true">
        <span>字</span>
      </div>
      <div><strong>字渡</strong><small>FontFerry</small></div>
    </div>
    <nav aria-label="主导航">
      {#each navigation as item}
        <button class:active={page === item.id} onclick={() => (page = item.id)}>
          <span>{item.symbol}</span>{item.label}
          {#if item.id === "activity" && updateCount}<b>{updateCount}</b>{/if}
        </button>
      {/each}
    </nav>
  </aside>

  <main>
    <header>
      <div>
        <h1>{navigation.find((item) => item.id === page)?.label}</h1>
      </div>
      <button class="primary" onclick={checkAllUpdates} disabled={checking}>
        {checking ? "正在检查…" : "检查字体更新"}
      </button>
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
        <article><small>已安装</small><strong>{data.installed.length}</strong><p>由字渡安装的字体</p></article>
        <article class="accent"><small>可直接更新</small><strong>{autoCount}</strong><p>字渡可以下载并安装</p></article>
        <article><small>需要手动更新</small><strong>{reminderCount}</strong><p>商业字体或没有公开下载</p></article>
        <article><small>可选字体</small><strong>{data.fonts.length}</strong><p>字渡目前支持的字体</p></article>
      </section>
      <section class="panel">
        <div class="panel-title"><div><h2>可用更新</h2><p>有新版本的字体会显示在这里</p></div></div>
        <div class="rows">
          {#each data.statuses.filter((item) => item.updateAvailable) as item}
            {@const font = data.fonts.find((candidate) => candidate.id === item.fontId)}
            {#if font}
              <button class="font-row" onclick={() => openFont(font)}>
                <span class="font-avatar">{font.name.slice(0, 1)}</span>
                <span class="grow"><strong>{font.name}</strong><small>{item.currentVersion ?? "未安装"} → {item.availableVersion}</small></span>
                <span class:reminder={item.deliveryPolicy === "notifyOnly"} class="pill">
                  {item.deliveryPolicy === "autoInstall" ? "可直接更新" : "手动更新"}
                </span>
                <span>›</span>
              </button>
            {/if}
          {:else}
            <div class="empty">暂时没有可用更新。点击右上角按钮可重新检查。</div>
          {/each}
        </div>
      </section>
    {:else if page === "catalog"}
      <section class="toolbar">
        <input aria-label="搜索字体" bind:value={query} placeholder="搜索字体、ID 或描述…" />
        <select aria-label="更新方式" bind:value={policy}>
          <option value="all">全部更新方式</option>
          <option value="autoInstall">可直接更新</option>
          <option value="notifyOnly">需要手动更新</option>
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
                {font.deliveryPolicy === "autoInstall" ? "可直接更新" : "手动更新"}
              </span>
            </div>
            <h2>{font.name}</h2>
            <code>{font.id}</code>
            <p>{font.description}</p>
            <footer>
              <span>{current ? `已安装 ${current.version}` : "未安装"}</span>
              {#if update?.updateAvailable}<strong>有更新</strong>{/if}
            </footer>
          </button>
        {/each}
      </section>
    {:else if page === "sources"}
      <section class="split">
        <article class="panel form">
          <div class="panel-title"><div><h2>添加字体</h2><p>保存到这台电脑，不会上传</p></div><span class="pill reminder">自定义</span></div>
          <label>检查方式
            <select bind:value={sourceKind}>
              <option value="github">GitHub Releases</option>
              <option value="metadata">检查网页是否变化（只提醒）</option>
            </select>
          </label>
          <div class="field-pair">
            <label>唯一名称<input bind:value={sourceId} placeholder="my-font" /></label>
            <label>字体名称<input bind:value={sourceName} placeholder="My Font" /></label>
          </div>
          {#if sourceKind === "github"}
            <label>GitHub 仓库<input bind:value={sourceRepository} placeholder="owner/repository" /></label>
            <label>下载文件匹配规则（高级）<input bind:value={sourceAssetPattern} /></label>
          {:else}
            <label>公开 HTTPS 地址<input bind:value={sourceHomepage} placeholder="https://example.com/font/" /></label>
          {/if}
          <button class="primary" onclick={saveSource} disabled={!sourceId || !sourceName}>检查并保存</button>
        </article>
        <article class="panel guidance">
          <h2>添加前请确认</h2>
          <ul>
            <li>下载地址必须是公开的 HTTPS 地址。</li>
            <li>字渡不会运行字体仓库中的脚本。</li>
            <li>每个选项只应匹配一个下载文件。</li>
            <li>请确认字体许可证允许你的使用方式。</li>
          </ul>
          <p>保存后，字渡会像检查内置字体一样检查这个字体。</p>
        </article>
      </section>
    {:else if page === "activity"}
      <section class="panel">
        <div class="panel-title"><div><h2>最近记录</h2><p>查看安装、更新和错误信息</p></div></div>
        <div class="timeline">
          {#each data.activities as item}
            <div class="event">
              <span class:bad={item.level === "error"} class:warn={item.level === "warning"}></span>
              <div><strong>{item.fontId ?? "FontFerry"}</strong><p>{item.message}</p></div>
              <time>{new Date(item.createdAt).toLocaleString()}</time>
            </div>
          {:else}
            <div class="empty">还没有记录。</div>
          {/each}
        </div>
      </section>
    {:else if page === "settings"}
      <section class="settings">
        <article class="panel">
          <h2>每日检查</h2>
          <label class="switch-row"><span><strong>每天自动检查字体更新</strong><small>即使字渡没有打开，也会按时检查</small></span><input type="checkbox" bind:checked={scheduleEnabled} /></label>
          <button class="primary" onclick={setSchedule}>保存</button>
        </article>
        <article class="panel">
          <h2>程序更新</h2>
          <div class="setting-row"><span><strong>更新通道</strong><small>稳定版更可靠；测试版可以提前体验新功能</small></span><select bind:value={updateChannel}><option value="stable">稳定版</option><option value="beta">测试版</option></select></div>
          <button class="primary" onclick={checkAppUpdate}>检查程序更新</button>
          {#if appUpdateAvailable}
            <button class="quiet" onclick={installAppUpdate}>安装更新</button>
          {/if}
          <p class="muted">通过系统软件商店安装的版本，请在系统中更新。</p>
        </article>
        <article class="panel">
          <h2>字体列表</h2>
          <div class="setting-row"><span><strong>内置字体</strong><small>随字渡提供，离线也可查看</small></span><span class="pill">可用</span></div>
          <div class="setting-row"><span><strong>在线字体列表</strong><small>更新失败时继续使用上次保存的内容</small></span></div>
          <button class="quiet" onclick={refreshCatalog}>更新字体列表</button>
        </article>
        <article class="panel">
          <h2>外观</h2>
          <div class="setting-row"><span><strong>颜色模式</strong><small>浅色与深色均使用黑白灰配色</small></span>
            <select bind:value={theme} onchange={changeTheme}>
              <option value="system">跟随系统</option>
              <option value="light">浅色</option>
              <option value="dark">深色</option>
            </select>
          </div>
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
        <div><small>当前版本</small><strong>{installed(selected.id)?.version ?? "未安装"}</strong></div>
        <div><small>最新版本</small><strong>{status(selected.id)?.availableVersion ?? "尚未检查"}</strong></div>
        <div><small>更新方式</small><strong>{selected.deliveryPolicy === "autoInstall" ? "字渡可安装" : "只提醒"}</strong></div>
        <div><small>许可证</small><strong>{selected.license.spdx ?? "商业/自定义"}</strong></div>
      </div>
      {#if selected.variants.length}
        <h3>选择字体包</h3>
        <p class="muted">名称来自字体作者。通常只需选择一个；多个包可能包含同名字体。</p>
        <div class="variants">
          {#each selected.variants as variant}
            <label class:selected={selectedVariants.includes(variant.id)}>
              <input type="checkbox" checked={selectedVariants.includes(variant.id)} onchange={() => toggleVariant(variant.id)} />
              <span><strong>{variant.name}</strong><small>{variant.description}</small></span>
            </label>
          {/each}
        </div>
      {/if}
      <div class="license-line"><span>许可协议：{selected.license.name}</span><a href={selected.license.url} target="_blank" rel="noreferrer">查看许可协议 ↗</a></div>
      {#if selected.deliveryPolicy === "notifyOnly"}
        <div class="manual-version">
          <label>当前安装版本<input bind:value={manualVersion} placeholder="例如 7.2.0" /></label>
          <button class="quiet" onclick={() => saveManualVersion(selected!)}>保存</button>
        </div>
      {/if}
      <div class="drawer-actions">
        <button class="quiet" onclick={() => check(selected!.id)} disabled={checking}>{checking ? "检查中…" : "检查更新"}</button>
        {#if selected.deliveryPolicy === "autoInstall"}
          <button class="primary" onclick={() => install(selected!)} disabled={!selectedVariants.length}>安装或更新</button>
        {:else}
          <a class="primary link" href={selected.homepage} target="_blank" rel="noreferrer">前往官方渠道</a>
        {/if}
      </div>
      {#if installed(selected.id)}
        <div class="danger-zone">
          {#if installed(selected.id)?.previous}<button onclick={() => rollback(selected!)}>恢复上一版本</button>{/if}
          <button onclick={() => remove(selected!)}>卸载</button>
        </div>
      {/if}
    </div>
  </div>
{/if}
