<!--
  FilePicker.svelte —— 左栏卡片 1：选择论文文件夹。
  Svelte 5 runes 风格（项目 svelte ^5.57）。
  纯展示组件：状态经 props 传入，动作经回调传出；不调用 Tauri invoke。
-->
<script lang="ts">
  import type { AuditRunStatus } from "../lib/types";

  interface Props {
    /** 当前已选文件夹路径；null 表示尚未选择。 */
    inputDir: string | null;
    status: AuditRunStatus;
    /** 已发现的 Word 文件数量；null 表示未知。 */
    fileCount: number | null;
    /** 点击「选择论文文件夹」时触发（父级接 stores.chooseInputDir）。 */
    onChoose: () => void;
    /**
     * 拖拽放置回调：参数为尽力提取的路径字符串（可能为空）。
     * 说明：Tauri v2 中原生文件拖拽需走窗口级 drag-drop 事件（webview 内
     * DataTransfer 不含真实路径），本阶段先预留此处理点，后续在 App/主进程接线。
     */
    onDropPath?: (path: string) => void;
  }

  let { inputDir, status, fileCount, onChoose, onDropPath }: Props = $props();

  const disabled = $derived(status === "running");
  const displayName = $derived(
    inputDir ? inputDir.split(/[\\/]/).filter(Boolean).pop() ?? inputDir : null,
  );

  let dragging = $state(false);

  function handleDrop(e: DragEvent): void {
    e.preventDefault();
    dragging = false;
    if (disabled) return;
    // 尽力而为地提取路径（浏览器沙箱下通常只有文件名；真实路径由 Tauri 事件提供）。
    const first = e.dataTransfer?.files?.[0];
    const path =
      // @ts-expect-error 浏览器扩展字段（Electron/Tauri 注入），标准 Web 不存在
      first?.path ?? first?.name ?? "";
    onDropPath?.(path);
  }
</script>

<section class="card bg-base-100 border border-base-200 shadow-sm" aria-label="论文文件夹">
  <div class="card-body gap-3">
    <div class="flex items-center justify-between gap-2">
      <h2 class="text-lg font-semibold text-base-content">论文文件夹</h2>
      {#if fileCount !== null && fileCount > 0}
        <span class="badge badge-info badge-sm">已发现 {fileCount} 个 Word 文件</span>
      {/if}
    </div>

    <label class="form-control w-full">
      <span class="label-text text-sm text-base-content/60 mb-1">当前选择的文件夹</span>
      <input
        type="text"
        readonly
        class="input input-bordered w-full truncate"
        placeholder={inputDir ? "" : "尚未选择文件夹"}
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
        选择论文文件夹
      {/if}
    </button>

    <!-- 拖拽区：视觉反馈 + 预留 onDrop 处理（真实路径待接入 Tauri drag-drop） -->
    <div
      role="button"
      tabindex={disabled ? -1 : 0}
      aria-label="将论文文件夹拖拽到此处"
      class={"rounded-lg border-2 border-dashed px-3 py-4 text-center text-sm transition-colors " +
        (disabled
          ? "border-base-300 text-base-content/40 cursor-not-allowed"
          : "border-base-300 text-base-content/60 hover:border-primary cursor-pointer " +
            (dragging ? "border-primary bg-base-200" : ""))}
      ondragover={(e: DragEvent) => { e.preventDefault(); if (!disabled) dragging = true; }}
      ondragleave={() => { dragging = false; }}
      ondrop={handleDrop}
      onkeydown={(e) => {
        if (!disabled && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          onChoose();
        }
      }}
    >
      或将文件夹拖拽到此处
    </div>

    <p class="text-sm text-base-content/60">请选择包含 .docx 论文的文件夹。</p>
  </div>
</section>
