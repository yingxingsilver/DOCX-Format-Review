<!--
  ExportBar.svelte —— P8D 审核完成后的导出栏。
  - 导出 HTML 报告 / CSV 清单；
  - 打开报告所在文件夹（revealItemInDir，权限不足时由状态层给中文提示）；
  - 展示已产出文件的路径并提供「打开文件」按钮（含后端生成的标注版 DOCX）。
  纯展示组件：所有动作经回调传出，组件内不调用 Tauri、不含业务逻辑。
-->
<script lang="ts">
  import type { ExportedFiles } from "../lib/types";

  interface Props {
    /** 是否允许导出（done/cancelled 且有结果时为 true）。 */
    enabled: boolean;
    /** 导出进行中：按钮禁用并显示 loading。 */
    busy: boolean;
    /** 勾选了「生成标注版 DOCX」且本次存在需修改文件（提示文案用）。 */
    annotatedPromised?: boolean;
    /** 已产出的文件清单（导出成功 / 后端标注生成后写入）。 */
    produced: ExportedFiles[];
    onExportHtml: () => void;
    onExportCsv: () => void;
    onOpenFolder: () => void;
    onOpenFile: (entry: ExportedFiles) => void;
  }

  let {
    enabled,
    busy,
    annotatedPromised = false,
    produced,
    onExportHtml,
    onExportCsv,
    onOpenFolder,
    onOpenFile,
  }: Props = $props();

  function kindLabel(kind: ExportedFiles["kind"]): string {
    switch (kind) {
      case "html":
        return "HTML 报告";
      case "csv":
        return "CSV 清单";
      default:
        return "标注版 DOCX";
    }
  }
</script>

{#if enabled}
  <section class="card bg-base-100 border border-base-200 shadow-sm" aria-label="导出检查结果">
    <div class="card-body gap-3">
      <h2 class="card-title text-lg font-semibold">导出检查结果</h2>

      <div class="flex flex-wrap gap-2">
        <button
          type="button"
          class="btn btn-primary btn-sm"
          disabled={!enabled || busy}
          onclick={onExportHtml}
        >
          {#if busy}<span class="loading loading-spinner loading-xs"></span>{/if}
          导出 HTML 报告
        </button>
        <button
          type="button"
          class="btn btn-secondary btn-sm"
          disabled={!enabled || busy}
          onclick={onExportCsv}
        >
          {#if busy}<span class="loading loading-spinner loading-xs"></span>{/if}
          导出 CSV 清单
        </button>
        <button
          type="button"
          class="btn btn-outline btn-sm"
          disabled={!enabled || busy || produced.length === 0}
          onclick={onOpenFolder}
        >
          打开报告所在文件夹
        </button>
      </div>

      {#if busy}
        <p class="text-sm text-base-content/60" role="status">
          <span class="loading loading-dots loading-xs mr-1"></span>正在生成，请稍候…
        </p>
      {/if}

      {#if produced.length > 0}
        <ul class="flex flex-col gap-1" aria-label="已生成的文件">
          {#each produced as item (item.abs_path)}
            <li class="flex items-center gap-2 text-sm">
              <span class="badge badge-success badge-sm shrink-0">{kindLabel(item.kind)}</span>
              <span class="truncate max-w-[420px]" title={item.abs_path}>{item.abs_path}</span>
              <button
                type="button"
                class="btn btn-ghost btn-xs shrink-0"
                disabled={busy}
                onclick={() => onOpenFile(item)}
              >
                打开文件
              </button>
            </li>
          {/each}
        </ul>
      {:else if annotatedPromised}
        <p class="text-sm text-base-content/60">
          标注版 DOCX 已随检查结果生成在论文文件夹中（文件名带“标注版”）。
        </p>
      {/if}

      <p class="text-sm text-base-content/60">报告保存在论文文件夹里，可直接发给老师或同学查看。</p>
    </div>
  </section>
{/if}
