<!--
  ProgressPanel.svelte —— 审核进度面板（P8C）。
  显示：总文件数 / 已处理数 / 当前文件名 / 百分比进度条 / 阶段文案 / 取消按钮。
  Svelte 5 runes；数据全部来自 props（App 从状态层下发），组件内不调用 Tauri invoke。
-->
<script lang="ts">
  interface Props {
    /** 已完成 / 总数；total<=0 时视为数据未就绪，显示 skeleton。 */
    progress: { completed: number; total: number };
    currentFileName?: string | null;
    /** 阶段文案，如“正在解析样式”；空串表示暂无阶段信息。 */
    stage?: string;
    /** 是否已完成（完成后显示“审核完成”，隐藏取消按钮）。 */
    finished?: boolean;
    /** 是否被取消。 */
    cancelled?: boolean;
    onCancel: () => void;
  }

  let {
    progress,
    currentFileName = null,
    stage = "",
    finished = false,
    cancelled = false,
    onCancel,
  }: Props = $props();

  const percent = $derived(
    progress.total > 0 ? Math.round((progress.completed / progress.total) * 100) : 0,
  );
  const remaining = $derived(Math.max(progress.total - progress.completed, 0));
</script>

<section class="card border border-line bg-white" aria-live="polite">
  <div class="card-body gap-5">
    <div class="flex items-baseline justify-between gap-3">
      <h2 class="font-heading text-[17px] font-semibold text-ink leading-none">
        {#if finished}
          审核完成
        {:else if cancelled}
          审核已停止
        {:else}
          正在审核
        {/if}
      </h2>
      <span class="text-sm text-subtle tabular-nums">
        {finished ? "已完成" : cancelled ? "已取消" : progress.total > 0 ? `${percent}%` : ""}
      </span>
    </div>

    <!-- 百分比进度条 -->
    <div>
      <div class="flex items-baseline justify-between mb-2">
        <span class="text-sm text-subtle">
          已处理 {progress.total > 0 ? progress.completed : "—"} /
          {progress.total > 0 ? progress.total : "—"} 个文件
        </span>
      </div>
      <progress
        id="audit-progress-bar"
        class="progress w-full {finished ? 'progress-success' : cancelled ? 'progress-warning' : 'progress-primary'}"
        value={progress.total > 0 ? percent : 0}
        max="100"
      ></progress>
    </div>

    <dl class="grid grid-cols-3 gap-x-6 text-sm">
      <div>
        <dt class="text-subtle">总文件数</dt>
        <dd class="pt-1 text-ink font-medium tabular-nums">
          {#if progress.total > 0}{progress.total}{:else}<span class="skeleton block h-4 w-8"></span>{/if}
        </dd>
      </div>
      <div>
        <dt class="text-subtle">已处理</dt>
        <dd class="pt-1 text-ink font-medium tabular-nums">
          {#if progress.total > 0}{progress.completed}{:else}<span class="skeleton block h-4 w-8"></span>{/if}
        </dd>
      </div>
      <div>
        <dt class="text-subtle">剩余</dt>
        <dd class="pt-1 text-ink font-medium tabular-nums">
          {#if progress.total > 0}{remaining}{:else}<span class="skeleton block h-4 w-8"></span>{/if}
        </dd>
      </div>
    </dl>

    <!-- 当前文件名 -->
    <div class="text-sm flex gap-2 min-w-0">
      <span class="text-subtle shrink-0">当前文件</span>
      {#if finished || cancelled}
        <span class="text-ink-2">—</span>
      {:else if currentFileName}
        <span class="inline-block max-w-[70%] truncate align-bottom text-ink-2" title={currentFileName}>
          {currentFileName}
        </span>
      {:else}
        <span class="skeleton inline-block h-4 w-1/3 align-middle"></span>
      {/if}
    </div>

    <!-- 阶段提示：一行文字，不用 steps 装饰 -->
    {#if !finished && !cancelled}
      {#if stage}
        <p class="text-sm text-subtle">{stage}…</p>
      {:else}
        <p class="text-sm text-subtle">准备中…</p>
      {/if}
    {:else if finished}
      <p class="text-sm text-subtle">结果已在下方列出。</p>
    {:else if cancelled}
      <p class="text-sm text-subtle">已完成部分的结果仍然可以查看。</p>
    {/if}

    <!-- 取消按钮：仅运行中显示，次要样式（主强调色留给开始 CTA） -->
    {#if !finished && !cancelled}
      <button type="button" class="btn btn-ghost self-start px-4" onclick={onCancel}>
        取消审核
      </button>
    {/if}
  </div>
</section>

<style>
  .bg-white { background-color: var(--bg); }
  .text-ink { color: var(--ink); }
  .text-ink-2 { color: var(--ink-2); }
  .text-subtle { color: var(--muted); }
  .border-line { border-color: var(--line); }
  .font-heading { font-family: var(--font-heading); }
</style>
