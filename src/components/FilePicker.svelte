<!--
  FilePicker.svelte —— 左栏卡片 1：选择论文文件夹。
  Svelte 5 runes 风格（项目 svelte ^5.57）。
  纯展示组件：状态经 props 传入，动作经回调传出；不调用 Tauri invoke。
-->
<script lang="ts">
  import type { AuditRunStatus } from "../lib/types";
  import { ICONS } from "../lib/icons";

  interface Props {
    /** 当前已选文件夹路径；null 表示尚未选择。 */
    inputDir: string | null;
    status: AuditRunStatus;
    /** 已发现的 Word 文件数量；null 表示未知。 */
    fileCount: number | null;
    /** 点击「选择论文文件夹」时触发（父级接 stores.chooseInputDir）。 */
    onChoose: () => void;
    /**
     * 原生文件/文件夹正被拖入窗口（Tauri drag-drop enter/leave 驱动，父级接 stores.draggingOver）。
     * 真实路径由 App 层统一监听 tauri://drag-drop 事件处理，本组件只负责高亮反馈。
     */
    dragging?: boolean;
  }

  let { inputDir, status, fileCount, onChoose, dragging = false }: Props = $props();

  const disabled = $derived(status === "running");
  const displayName = $derived(
    inputDir ? inputDir.split(/[\\/]/).filter(Boolean).pop() ?? inputDir : null,
  );
</script>

<section class="card border border-line bg-white" aria-label="论文文件夹">
  <div class="card-body gap-4">
    <div class="flex items-center justify-between gap-2">
      <h2 class="font-heading text-[17px] font-semibold text-ink leading-none">论文文件夹</h2>
      {#if fileCount !== null && fileCount > 0}
        <span class="text-sm text-subtle">{fileCount} 个 Word 文件</span>
      {/if}
    </div>

    <label class="form-control w-full">
      <span class="text-sm text-subtle mb-1.5 block">当前选择的文件夹</span>
      <input
        type="text"
        readonly
        class="input input-bordered w-full truncate"
        placeholder={inputDir ? "" : "尚未选择"}
        value={displayName ?? ""}
        title={inputDir ?? "尚未选择文件夹"}
        aria-label="当前选择的论文文件夹路径"
      />
    </label>

    <button
      type="button"
      class="btn btn-primary w-full"
      disabled={disabled}
      onclick={onChoose}
    >
      {#if disabled}
        <span class="loading loading-spinner loading-xs" aria-hidden="true"></span>
        审核进行中
      {:else}
        <span class="icon" style="--icon-size:16px" aria-hidden="true">{@html ICONS.folder}</span>
        选择论文文件夹
      {/if}
    </button>

    <!-- 拖拽区：drop 本身由 Tauri 窗口级 drag-drop 事件处理（dragDropEnabled 会拦截 webview 默认行为），
         这里 preventDefault 防止浏览器导航到拖放内容，高亮状态来自父级传入的 dragging。 -->
    <div
      role="button"
      tabindex={disabled ? -1 : 0}
      aria-label="将论文文件夹拖拽到此处"
      class={"rounded-card border border-dashed px-4 py-5 text-center text-sm transition-colors " +
        (disabled
          ? "border-line text-subtle cursor-not-allowed"
          : "border-line text-subtle hover:border-accent hover:text-ink cursor-pointer " +
            (dragging ? "border-accent bg-surface text-ink" : ""))}
      ondragover={(e: DragEvent) => e.preventDefault()}
      ondrop={(e: DragEvent) => e.preventDefault()}
      onkeydown={(e) => {
        if (!disabled && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          onChoose();
        }
      }}
    >
      <span class="icon mb-1" style="--icon-size:18px" aria-hidden="true">{@html ICONS.upload}</span>
      <div>或将文件夹拖拽到此处</div>
    </div>

    <p class="text-sm text-subtle">仅支持 .docx；.doc 请先用 Word 另存为 .docx。</p>
  </div>
</section>

<style>
  .bg-white {
    background-color: var(--bg);
  }
  .bg-surface {
    background-color: var(--surface);
  }
  .text-ink {
    color: var(--ink);
  }
  .text-subtle {
    color: var(--muted);
  }
  .border-line {
    border-color: var(--line);
  }
  .border-accent,
  .hover\:border-accent:hover {
    border-color: var(--accent);
  }
  .hover\:text-ink:hover {
    color: var(--ink);
  }
  .rounded-card {
    border-radius: var(--radius-card);
  }
  .font-heading {
    font-family: var(--font-heading);
  }
</style>
