<!--
  ActionPanel.svelte —— 左栏卡片 3：审核选项与操作按钮。
  纯展示组件：状态经 props 传入，动作经回调传出；不调用 Tauri invoke。
-->
<script lang="ts">
  import type { AuditRunStatus } from "../lib/types";
  import { ICONS } from "../lib/icons";

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
    /** 导出功能是否已接入本界面（有检查结果后开放）。 */
    exportsReady?: boolean;
    onIncludeSubdirsChange: (v: boolean) => void;
    onGenerateAnnotatedChange: (v: boolean) => void;
    onStart: () => void;
    onCancel: () => void;
    onClear: () => void;
    onExportHtml: () => void;
    onExportCsv: () => void;
    /** 在系统文件管理器中打开输出文件夹（报告与标注版 DOCX 所在处）。 */
    onOpenAnnotatedFolder?: () => void;
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
    onOpenAnnotatedFolder,
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

<section class="card border border-line bg-white" aria-label="审核选项与操作">
  <div class="card-body gap-4">
    <h2 class="font-heading text-[17px] font-semibold text-ink leading-none">审核选项</h2>

    <!-- 选项区 -->
    <div class="flex flex-col gap-3">
      <label class="flex items-center justify-between gap-3 cursor-pointer" for="opt-subdirs">
        <span class="text-[15px] text-ink-2">包含子文件夹</span>
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

      <div>
        <label class="flex items-center justify-between gap-3 cursor-pointer" for="opt-annotated">
          <span class="text-[15px] text-ink-2">生成标注版 DOCX</span>
          <input
            id="opt-annotated"
            type="checkbox"
            class="toggle toggle-primary"
            checked={generateAnnotatedDocx}
            disabled={optionsDisabled}
            onchange={(e: Event) =>
              onGenerateAnnotatedChange((e.currentTarget as HTMLInputElement).checked)}
          />
        </label>
        <p class="text-sm text-subtle pt-1">在原文中标出问题位置，处理速度稍慢。</p>
      </div>
    </div>

    <div class="divider my-0"></div>

    <!-- 操作按钮区（Tab 顺序：开始 → 取消 → 清空） -->
    {#if !running}
      <button
        type="button"
        class="btn btn-primary w-full h-12 text-base"
        disabled={!canStart}
        title={canStart ? "开始批量审核" : "请先在上方选择论文文件夹"}
        onclick={onStart}
      >
        <span class="icon" style="--icon-size:16px" aria-hidden="true">{@html ICONS.play}</span>
        开始批量审核
      </button>
      {#if !canStart}
        <p class="text-sm text-subtle -mt-2">先选择论文文件夹，再开始审核。</p>
      {/if}
    {:else}
      <button type="button" class="btn btn-primary w-full h-12 text-base" disabled>
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

    <!-- 结果产物入口：报告 / 清单导出，以及打开输出文件夹（标注版 DOCX 在这里） -->
    {#if status === "done" || status === "cancelled"}
      <div class="grid grid-cols-2 gap-2">
        <button
          type="button"
          class="btn btn-outline btn-sm w-full"
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
          导出 CSV 清单
        </button>
      </div>
      {#if onOpenAnnotatedFolder}
        <button
          type="button"
          class="btn btn-ghost btn-sm w-full -mt-1"
          disabled={!exportsReady}
          onclick={onOpenAnnotatedFolder}
        >
          打开输出文件夹
        </button>
      {/if}
      <p class="text-sm text-subtle -mt-2">
        报告与标注版 DOCX 保存在论文文件夹下的“格式检查输出”里。
      </p>
    {/if}

    <!-- 状态提示（aria-live 便于读屏播报） -->
    <div aria-live="polite">
      {#if status === "error" && errorMessage}
        <div role="alert" class="alert alert-error py-2 text-sm">
          <span>{errorMessage}</span>
        </div>
      {:else}
        <p class="text-sm text-subtle">{statusText[status]}</p>
      {/if}
    </div>
  </div>
</section>

<style>
  .bg-white {
    background-color: var(--bg);
  }
  .text-ink {
    color: var(--ink);
  }
  .text-ink-2 {
    color: var(--ink-2);
  }
  .text-subtle {
    color: var(--muted);
  }
  .border-line {
    border-color: var(--line);
  }
  .font-heading {
    font-family: var(--font-heading);
  }
</style>
