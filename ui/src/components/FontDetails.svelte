<script lang="ts">
  import type { FontDefinition, InstalledFont, UpdateStatus } from "../types";
  import type { Operation } from "../operations";
  export let selected: FontDefinition | null;
  export let selectedVariants: string[];
  export let manualVersion: string;
  export let operation: Operation | null;
  export let licenseAccepted: boolean;
  export let message: string;
  export let error: string;
  export let installedFonts: InstalledFont[];
  export let statuses: UpdateStatus[];
  export let toggleVariant: (id: string) => void;
  export let check: (id: string) => Promise<void>;
  export let install: (font: FontDefinition) => Promise<void>;
  export let remove: (font: FontDefinition) => Promise<void>;
  export let rollback: (font: FontDefinition) => Promise<void>;
  export let saveManualVersion: (font: FontDefinition) => Promise<void>;
  $: current = installedFonts.find((item) => item.fontId === selected?.id);
  $: update = statuses.find((item) => item.fontId === selected?.id);
  $: savedVariants = current?.variantIds?.length ? current.variantIds : selected?.variants.filter((variant) => variant.default).map((variant) => variant.id) ?? [];
  $: variantsChanged = !!current && (selectedVariants.length !== savedVariants.length || selectedVariants.some((id) => !savedVariants.includes(id)));
  $: installLabel = !current ? "安装字体" : update?.updateAvailable ? "更新字体" : variantsChanged ? "更改字体包" : "重新安装";
</script>

{#if selected}
  <div class="backdrop" role="presentation" onclick={(event) => event.target === event.currentTarget && (selected = null)}>
    <div class="drawer" role="dialog" aria-modal="true" aria-label={selected.name}>
      <button class="close" aria-label="关闭" onclick={() => (selected = null)}>×</button>
      <p class="eyebrow">{selected.id}</p>
      <h1>{selected.name}</h1>
      <p class="lead">{selected.description}</p>
      {#if error}<div class="notice error" role="alert">{error}</div>{/if}
      {#if message}<div class="notice success" role="status">{message}</div>{/if}
      {#if operation}<p role="status" class="muted">{operation.title}。{operation.detail}</p>{/if}
      <div class="detail-grid">
        <div><small>当前版本</small><strong>{current?.manualVersion ?? current?.version ?? update?.currentVersion ?? "尚无安装记录"}</strong></div>
        <div><small>最新版本</small><strong>{update?.availableVersion ?? "尚未检查"}</strong></div>
        <div><small>更新方式</small><strong>{selected.deliveryPolicy === "autoInstall" ? "字渡可安装" : "只提醒"}</strong></div>
        <div><small>许可证</small><strong>{selected.license.spdx ?? "商业/自定义"}</strong></div>
      </div>
      {#if current}<p class="muted">字渡管理 {current.ownedFiles?.length ?? 0} 个字体文件，仅作用于当前用户。</p>{/if}
      {#if update?.fromCache}
        <p class="muted">缓存的远程版本信息；{update?.checkedAt ? `检查时间：${new Date(update!.checkedAt!).toLocaleString()}` : "检查时间未知"}。本地安装状态已刷新。</p>
      {/if}
      {#if selected.variants.length}
        <h3>选择字体包</h3>
        <p class="muted">名称来自字体作者。通常只需选择一个；多个包可能包含同名字体。</p>
        <div class="variants">
          {#each selected.variants as variant}
            <label class:selected={selectedVariants.includes(variant.id)}>
              <input type="checkbox" disabled={operation !== null} checked={selectedVariants.includes(variant.id)} onchange={() => toggleVariant(variant.id)} />
              <span><strong>{variant.name}</strong><small>{variant.description}</small></span>
            </label>
          {/each}
        </div>
      {/if}
      <div class="license-line"><span>许可协议：{selected.license.name}</span><a href={selected.license.url} target="_blank" rel="noreferrer">查看许可协议 ↗</a></div>
      {#if selected.license.requiresAcceptance && selected.deliveryPolicy === "autoInstall"}
        <label class="license-consent"><input type="checkbox" bind:checked={licenseAccepted} disabled={operation !== null} /><span>我已阅读并同意此字体的许可协议</span></label>
      {/if}
      {#if selected.deliveryPolicy === "notifyOnly"}
        <div class="manual-version">
          <label>当前安装版本<input bind:value={manualVersion} placeholder="例如 7.2.0" /></label>
          <button class="quiet" onclick={() => saveManualVersion(selected!)} disabled={operation !== null}>保存</button>
        </div>
      {/if}
      <div class="drawer-actions">
        <button class="quiet" onclick={() => check(selected!.id)} disabled={operation !== null}>
          {#if operation?.kind === "check-fonts"}<span class="button-spinner"></span>{/if}
          {operation?.kind === "check-fonts" ? "检查中…" : "检查更新"}
        </button>
        {#if selected.deliveryPolicy === "autoInstall"}
          <button class="primary" onclick={() => install(selected!)} disabled={!selectedVariants.length || operation !== null || (selected.license.requiresAcceptance && !licenseAccepted)}>
            {#if operation?.kind === "install-font"}<span class="button-spinner"></span>{/if}
            {operation?.kind === "install-font" ? "正在安装…" : installLabel}
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
