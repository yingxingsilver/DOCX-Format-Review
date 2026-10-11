<!--
  ActionPanel.svelte —— 左栏卡片 3：审核选项与操作按钮。
  纯展示组件：状态经 props 传入，动作经回调传出；不调用 Tauri invoke。
-->
<script lang="ts">
  import type { AuditRunStatus } from "../lib/types";

  interface Props {
    status: AuditRunStatus;
    /** 未选择文件夹时禁用「开始批量审核」。 */
    canStart: boolean;
    includeSubdirs: boolean;
    generateAnnotatedDocx: boolean;
    /** 是否存在可清空的结果（done/error/cancelled 或已有摘要）。 */
    hasResults: boolean;
    /** error 状态的中文提示（alert-error 展示）。 */
    errorMessage: string | null;
    /** 导出功能是否已接入本界面（P8B 占位：false → 禁用并提示）。 */
    exportsReady?: boolean;
    onIncludeSubdirsChange: (v: boolean) => void;
    onGenerateAnnotatedChange: (v: boolean) => void;
    onStart: () => void;
    onCancel: () => void;
    onClear: () => void;
    onExportHtml: () => void;
    onExportCsv: () => void;
  }

  let {
    status,
    canStart,
    includeSubdirs,
    generateAnnotatedDocx,
    hasResults,
    errorMessage,
    exportsReady = false,
    onIncludeSubdirsChange,
    onGenerateAnnotatedChange,
    onStart,
    onCancel,
    onClear,
    onExportHtml,
    onExportCsv,
  }: Props = $props();

  const running = $derived(status === "running");
  const optionsDisabled = $derived(running);

  const statusText: Record<AuditRunStatus, string> = {
    idle: "准备就绪",
    running: "正在审核，请稍候",
    done: "审核完成",
    error: "审核失败，请检查提示",
    cancelled: "已取消审核",
  };
</script>

<section class="card bg-base-100 border border-base-200 shadow-sm" aria-label="审核选项与操作">
  <div class="card-body gap-3">
    <h2 class="text-lg font-semibold text-base-content">审核选项与操作</h2>

    <!-- 选项区 -->
    <div class="flex items-center justify-between gap-2">
      <label class="label cursor-pointer justify-end gap-3 w-full" for="opt-subdirs">
        <span class="label-text">包含子文件夹</span>
        <input
          id="opt-subdirs"
          type="checkbox"
          class="toggle toggle-primary"
          checked={includeSubdirs}
          disabled={optionsDisabled}
          onchange={(e: Event) =>
            onIncludeSubdirsChange((e.currentTarget as HTMLInputElement).checked)}
        />
      </label>
    </div>

    <div>
      <label class="label cursor-pointer justify-start gap-3 p-0" for="opt-annotated">
        <input
          id="opt-annotated"
          type="checkbox"
          class="toggle toggle-primary"
          checked={generateAnnotatedDocx}
          disabled={optionsDisabled}
          onchange={(e: Event) =>
            onGenerateAnnotatedChange((e.currentTarget as HTMLInputElement).checked)}
        />
        <span class="label-text">生成标注版 DOCX</span>
      </label>
      <p class="text-sm text-base-content/60 pl-1 pt-1">
        会在原文件基础上标记问题位置，处理速度可能稍慢。
      </p>
    </div>

    <div class="divider my-2"></div>

    <!-- 操作按钮区（Tab 顺序：开始 → 取消 → 清空） -->
    {#if !running}
      <button
        type="button"
        class="btn btn-primary w-full"
        disabled={!canStart}
        title={canStart ? "开始批量审核" : "请先在上方选择论文文件夹"}
        onclick={onStart}
      >
        开始批量审核
      </button>
      {#if !canStart}
        <p class="text-sm text-base-content/60 -mt-1">请先选择论文文件夹后开始。</p>
      {/if}
    {:else}
      <button type="button" class="btn btn-primary w-full" disabled>
        <span class="loading loading-spinner loading-sm" aria-hidden="true"></span>
        审核进行中
      </button>
      <button type="button" class="btn btn-error w-full" onclick={onCancel}>
        取消审核
      </button>
    {/if}

    <button
      type="button"
      class="btn btn-ghost w-full"
      disabled={running || !hasResults}
      onclick={onClear}
    >
      清空结果
    </button>

    <!-- 导出占位：不阻塞主流程 -->
    {#if status === "done"}
      <div class="grid grid-cols-2 gap-2">
        <button
          type="button"
          class="btn btn-secondary btn-sm w-full"
          disabled={!exportsReady}
          onclick={onExportHtml}
        >
          导出 HTML 报告
        </button>
        <button
          type="button"
          class="btn btn-outline btn-sm w-full"
          disabled={!exportsReady}
          onclick={onExportCsv}
        >
          导出 Excel/CSV 清单
        </button>
      </div>
      {#if !exportsReady}
        <p class="text-sm text-base-content/60">报告导出将在结果页开放。</p>
      {/if}
    {/if}

    <!-- 状态提示（aria-live 便于读屏播报） -->
    <div aria-live="polite">
      {#if status === "error" && errorMessage}
        <div role="alert" class="alert alert-error py-2 text-sm">
          <span>{errorMessage}</span>
        </div>
      {:else}
        <p class="text-sm text-base-content/60">{statusText[status]}</p>
      {/if}
    </div>
  </div>
</section>
