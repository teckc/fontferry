<script lang="ts">
  import type { FontDefinition, InstallCandidate, InstalledFont, UpdateStatus, Variant } from "../types";
  import type { Operation } from "../operations";
  export let selected: FontDefinition | null;
  export let selectedVariants: string[];
  export let manualVersion: string;
  export let operation: Operation | null;
  export let candidate: InstallCandidate | null;
  export let installFailureKind: string;
  export let failedChecks: string[];
  export let licenseAccepted: boolean;
  export let message: string;
  export let error: string;
  export let installedFonts: InstalledFont[];
  export let statuses: UpdateStatus[];
  export let toggleVariant: (id: string) => void;
  export let check: (id: string) => Promise<void>;
  export let previewInstall: (font: FontDefinition) => Promise<void>;
  export let install: (font: FontDefinition) => Promise<void>;
  export let configureNetwork: (fontId: string) => void;
  export let remove: (font: FontDefinition) => Promise<void>;
  export let rollback: (font: FontDefinition) => Promise<void>;
  export let saveManualVersion: (font: FontDefinition) => Promise<void>;
  $: current = installedFonts.find((item) => item.fontId === selected?.id);
  $: update = statuses.find((item) => item.fontId === selected?.id);
  $: desktopVariants = selected?.variants.filter((variant) => !variant.webOnly) ?? [];
  $: savedVariants = current?.variantIds?.length ? current.variantIds : selected?.variants.filter((variant) => variant.default || desktopVariants.length === 1).map((variant) => variant.id) ?? [];
  $: variantsChanged = !!current && (selectedVariants.length !== savedVariants.length || selectedVariants.some((id) => !savedVariants.includes(id)));
  $: recommended = desktopVariants.filter((variant) => !variant.advanced);
  $: advanced = desktopVariants.filter((variant) => variant.advanced);
  $: pageTracked = selected?.versionProvider.type === "httpFingerprint";
  $: installLabel = !current ? "安装字体" : update?.updateAvailable ? "更新字体" : variantsChanged ? "更改字体包" : "重新安装";
</script>

{#snippet packageOption(variant: Variant)}
  <label class:selected={selectedVariants.includes(variant.id)}>
    <input type="checkbox" disabled={operation !== null} checked={selectedVariants.includes(variant.id)} onchange={() => toggleVariant(variant.id)} />
    <span><strong>{variant.name}</strong>{#if variant.description}<small>{variant.description}</small>{/if}</span>
  </label>
{/snippet}

{#if selected}
  <div class="backdrop" role="presentation" onclick={(event) => !operation && event.target === event.currentTarget && (selected = null)}>
    <div class="drawer" role="dialog" aria-modal="true" aria-label={selected.name}>
      <button class="close" aria-label="关闭" disabled={operation !== null} onclick={() => (selected = null)}>×</button>
      <p class="eyebrow">{selected.id}</p>
      <h1>{selected.name}</h1>
      <p class="lead">{selected.description}</p>
      {#if error}<div class="notice error" role="alert">{error}</div>{/if}
      {#if error && installFailureKind === "network"}
        <button class="quiet" onclick={() => configureNetwork(selected!.id)} disabled={operation !== null}>打开网络设置</button>
      {/if}
      {#if message}<div class="notice success" role="status">{message}</div>{/if}
      {#if operation}<p role="status" class="muted">{operation.title}。{operation.detail}</p>{/if}
      <div class="detail-grid">
        {#if pageTracked}
          <div><small>官方下载页</small><strong>{failedChecks.includes(selected.id) ? "检查失败" : !update?.availableVersion ? "尚未检查" : !update.currentVersion ? "已检查" : update.updateAvailable ? "有变化" : "未检测到变化"}</strong></div>
        {:else}
          <div><small>当前版本</small><strong>{current?.manualVersion ?? current?.version ?? update?.currentVersion ?? "尚无安装记录"}</strong></div>
          <div><small>{update?.fromCache ? "上次查到的版本" : "可用版本"}</small><strong>{update?.availableVersion ?? (failedChecks.includes(selected.id) ? "检查失败" : update ? "无可用版本" : "尚未检查")}</strong></div>
        {/if}
        <div><small>更新方式</small><strong>{selected.deliveryPolicy === "autoInstall" ? "字渡可安装" : "只提醒"}</strong></div>
        <div><small>许可证</small><strong>{selected.license.spdx ?? "商业/自定义"}</strong></div>
      </div>
      {#if selected.versionProvider.channel === "prerelease"}<p class="muted">字体预发布</p>{/if}
      {#if update?.packageIssue}<p class="muted">该发布的当前规格不可用。{update.packageIssue}</p>{/if}
      {#if current}<p class="muted">字渡管理 {current.ownedFiles?.length ?? 0} 个字体文件，仅作用于当前用户。</p>{/if}
      {#if update?.fromCache}
        <p class="muted">缓存的远程版本信息；{update?.checkedAt ? `检查时间：${new Date(update!.checkedAt!).toLocaleString()}` : "检查时间未知"}。本地安装状态已刷新。</p>
      {/if}
      {#if desktopVariants.length > 1 || desktopVariants.some((variant) => !selectedVariants.includes(variant.id))}
        <h3>选择字体包</h3>
        <div class="variants">
          {#each recommended as variant (variant.id)}
            {@render packageOption(variant)}
          {/each}
        </div>
        {#if advanced.length}
          {#key selected.id}
            <details open={advanced.some((variant) => selectedVariants.includes(variant.id))}>
              <summary>高级选项</summary>
              <div class="variants">
                {#each advanced as variant (variant.id)}
                  {@render packageOption(variant)}
                {/each}
              </div>
            </details>
          {/key}
        {/if}
      {/if}
      {#if selected.variants.some((variant) => variant.webOnly)}
        <a class="quiet" href={selected.homepage} target="_blank" rel="noreferrer">查看官方 WOFF2 网页字体</a>
      {/if}
      <div class="license-line"><span>许可协议：{selected.license.name}</span><a href={selected.license.url} target="_blank" rel="noreferrer">查看许可协议 ↗</a></div>
      {#if selected.license.requiresAcceptance && selected.deliveryPolicy === "autoInstall"}
        <label class="license-consent"><input type="checkbox" bind:checked={licenseAccepted} disabled={operation !== null} /><span>我已阅读并同意此字体的许可协议</span></label>
      {/if}
      {#if pageTracked && update?.availableVersion && !update.fromCache}
        <button class="quiet" disabled={operation !== null || update.availableVersion === update.currentVersion} onclick={() => {
          manualVersion = update!.availableVersion!;
          void saveManualVersion(selected!);
        }}>记录本次下载页状态</button>
      {:else if selected.deliveryPolicy === "notifyOnly" && !pageTracked}
        <div class="manual-version">
          <label>当前安装版本<input bind:value={manualVersion} placeholder="例如 7.2.0" /></label>
          <button class="quiet" onclick={() => saveManualVersion(selected!)} disabled={operation !== null}>保存</button>
        </div>
      {/if}
      {#if candidate}
        <div class="license-line">
          <strong>确认安装 {candidate.version}</strong>
          <span>{candidate.assetNames.join("、")}{#if candidate.downloadBytes != null} · {new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 1 }).format(candidate.downloadBytes / 1048576)} MiB{/if}</span>
        </div>
        {#if candidate.archiveMembers?.length}
          <p class="muted">将安装 {candidate.archiveMembers.join("、")}</p>
        {/if}
      {/if}
      <div class="drawer-actions">
        <button class="quiet" onclick={() => check(selected!.id)} disabled={operation !== null}>
          {#if operation?.kind === "check-fonts"}<span class="button-spinner"></span>{/if}
          {operation?.kind === "check-fonts" ? "检查中…" : "检查更新"}
        </button>
        {#if selected.deliveryPolicy === "autoInstall"}
          <button class="primary" onclick={() => candidate ? install(selected!) : previewInstall(selected!)} disabled={!selectedVariants.length || operation !== null || installFailureKind === "recovery" || (selected.license.requiresAcceptance && !licenseAccepted)}>
            {#if operation?.kind === "install-font"}<span class="button-spinner"></span>{/if}
            {operation?.kind === "install-font" ? "正在安装…" : candidate ? "确认安装" : installLabel}
          </button>
        {:else}
          <a class="primary link" href={selected.homepage} target="_blank" rel="noreferrer">前往官方渠道</a>
        {/if}
      </div>
      {#if current}
        <div class="danger-zone">
          {#if current?.previous}<button disabled={operation !== null} onclick={() => rollback(selected!)}>恢复至 {current.previous.version}</button>{/if}
          <button disabled={operation !== null} onclick={() => remove(selected!)}>卸载字渡管理的文件</button>
        </div>
      {/if}
    </div>
  </div>
{/if}
