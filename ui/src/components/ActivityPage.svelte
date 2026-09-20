<script lang="ts">
  import type { Activity } from "../types";
  export let activities: Activity[];
</script>
      <section class="panel">
        <div class="panel-title"><div><h2>最近记录</h2><p>查看安装、更新和错误信息</p></div></div>
        <div class="timeline">
          {#each activities as item}
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
