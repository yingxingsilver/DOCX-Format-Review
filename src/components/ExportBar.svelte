<!--
  ExportBar.svelte —— P8D 导出与打开闭环工具条。
  Svelte 5 runes 风格（项目 svelte ^5.57）。纯展示组件：状态经 props 传入，动作经回调传出。

  要求（P8D）：
  - 审核完成（done / cancelled 且有结果）后显示：
    · 导出 HTML 报告 / 导出 CSV 清单（导出中显示 loading，按钮禁用）；
    · 打开报告所在文件夹；
    · 打开标注版 DOCX（仅当后端生成了标注副本时出现）；
  - 导出成功后显示保存路径，并提供「打开文件」按钮；
  - opener 权限不足等错误由父级以中文提示条呈现（本组件不抛错、不崩溃）。
-->
<script lang="ts">
  import type { AuditRunStatus } from "../lib/types";

  interface Props {
    status: AuditRunStatus;
    /** 是否已有可导出的检查结果（summary !== null）。 */
    hasResults: boolean;
    /** 导出进行中（任一导出按钮触发后置 true）。 */
    exporting?: boolean;
    /** 最近一次成功导出的路径（键 "html" | "csv"），null 表示尚未导出。 */
    htmlPath?: string | null;
    csvPath?: string | null;
    /** 是否已生成标注版 DOCX（后端返回的副本清单非空）。 */
    hasAnnotated?: boolean;
    /** 标注副本数量（用于按钮文案）。 */
    annotatedCount?: number;
    onExportHtml: () => void;
    onExportCsv: () => void;
    /** 打开指定路径（报告文件 / 标注 DOCX）。 */
    onOpenFile: (path: string) => void;
    onOpenFolder: () => void;
    /** 打开标注版 DOCX（第一个；多个时父级在详情中提供逐个打开）。 */
    onOpenAnnotated?: () => void;
  }

  let {
    status,
    hasResults,
    exporting = false,
    htmlPath = null,
    csvPath = null,
    hasAnnotated = false,
    annotatedCount = 0,
    onExportHtml,
    onExportCsv,
    onOpenFile,
    onOpenFolder,
    onOpenAnnotated = () => {},
  }: Props = $props();

  /** done / cancelled（取消也保留部分结果可导出）且已有结果时显示整条工具栏。 */
  const visible = $derived(hasResults && (status === "done" || status === "cancelled"));
</script>

{#if visible}
  <section class="card bg-base-100 border border-base-200 shadow-sm" aria-label="导出与打开">
    <div class="card-body gap-3">
      <h2 class="text-lg font-semibold text-base-content">查看结果与导出</h2>

      <!-- 主操作按钮行 -->
      <div class="flex flex-wrap gap-2">
        <button
          type="button"
          class="btn btn-primary btn-sm gap-2"
          disabled={exporting}
          onclick={onExportHtml}
        >
          {#if exporting}
            <span class="loading loading-spinner loading-xs" aria-hidden="true"></span>
            正在导出…
          {:else}
            导出 HTML 报告
          {/if}
        </button>

        <button
          type="button"
          class="btn btn-secondary btn-sm gap-2"
          disabled={exporting}
          onclick={onExportCsv}
        >
          {#if exporting}
            <span class="loading loading-spinner loading-xs" aria-hidden="true"></span>
            正在导出…
          {:else}
            导出 CSV 清单
          {/if}
        </button>

        <button type="button" class="btn btn-ghost btn-sm border border-base-300" onclick={onOpenFolder}>
          打开报告所在文件夹
        </button>

        {#if hasAnnotated}
          <button type="button" class="btn btn-accent btn-sm" onclick={onOpenAnnotated}>
            打开标注版 DOCX{annotatedCount > 1 ? `（共 ${annotatedCount} 个）` : ""}
          </button>
        {/if}
      </div>

      <!-- 导出成功后的路径 + 打开文件 -->
      {#if htmlPath || csvPath}
        <ul class="space-y-1 text-sm" aria-label="已导出的文件">
          {#if htmlPath}
            <li class="flex items-center gap-2">
              <span class="badge badge-success badge-sm shrink-0">HTML 报告</span>
              <code class="truncate flex-1 text-base-content/70" title={htmlPath}>{htmlPath}</code>
              <button type="button" class="btn btn-ghost btn-xs" onclick={() => onOpenFile(htmlPath)}>
                打开文件
              </button>
            </li>
          {/if}
          {#if csvPath}
            <li class="flex items-center gap-2">
              <span class="badge badge-success badge-sm shrink-0">CSV 清单</span>
              <code class="truncate flex-1 text-base-content/70" title={csvPath}>{csvPath}</code>
              <button type="button" class="btn btn-ghost btn-xs" onclick={() => onOpenFile(csvPath)}>
                打开文件
              </button>
            </li>
          {/if}
        </ul>
        <p class="text-xs text-base-content/50">
          如果点击「打开文件」没有反应，请到上面的保存位置手动双击文件。
        </p>
      {:else}
        <p class="text-xs text-base-content/50">
          报告与清单会保存在论文文件夹中，导出后可在这里直接打开。
        </p>
      {/if}
    </div>
  </section>
{/if}
