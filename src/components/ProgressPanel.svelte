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

<section class="card bg-base-100 border border-base-200 shadow-sm" aria-live="polite">
  <div class="card-body gap-4">
    <div class="flex items-center justify-between">
      <h2 class="card-title text-lg font-semibold text-base-content">
        {#if finished}
          审核完成
        {:else if cancelled}
          审核已停止
        {:else}
          正在审核
        {/if}
      </h2>
      <span class="badge {finished ? 'badge-success' : cancelled ? 'badge-warning' : 'badge-info'} badge-outline">
        {finished ? "已完成" : cancelled ? "已取消" : `${percent}%`}
      </span>
    </div>

    <!-- 百分比进度条 -->
    <div class="form-control">
      <label class="label" for="audit-progress-bar">
        <span class="label-text">
          已处理 {progress.total > 0 ? progress.completed : "—"} /
          {progress.total > 0 ? progress.total : "—"} 个文件
        </span>
        <span class="label-text">{progress.total > 0 ? `${percent}%` : ""}</span>
      </label>
      <progress
        id="audit-progress-bar"
        class="progress w-full {finished ? 'progress-success' : cancelled ? 'progress-warning' : 'progress-primary'}"
        value={progress.total > 0 ? percent : 0}
        max="100"
      ></progress>
    </div>

    <dl class="grid grid-cols-3 gap-3 text-sm">
      <div>
        <dt class="text-base-content/60">总文件数</dt>
        <dd class="pt-1 font-medium">
          {#if progress.total > 0}{progress.total}{:else}<span class="skeleton block h-4 w-8"></span>{/if}
        </dd>
      </div>
      <div>
        <dt class="text-base-content/60">已处理</dt>
        <dd class="pt-1 font-medium">
          {#if progress.total > 0}{progress.completed}{:else}<span class="skeleton block h-4 w-8"></span>{/if}
        </dd>
      </div>
      <div>
        <dt class="text-base-content/60">剩余</dt>
        <dd class="pt-1 font-medium">
          {#if progress.total > 0}{remaining}{:else}<span class="skeleton block h-4 w-8"></span>{/if}
        </dd>
      </div>
    </dl>

    <!-- 当前文件名 -->
    <div class="text-sm">
      <span class="text-base-content/60">当前文件：</span>
      {#if finished || cancelled}
        <span class="font-medium">—</span>
      {:else if currentFileName}
        <span class="inline-block max-w-[70%] truncate align-bottom font-medium" title={currentFileName}>
          {currentFileName}
        </span>
      {:else}
        <span class="skeleton inline-block h-4 w-1/3 align-middle"></span>
      {/if}
    </div>

    <!-- 阶段提示 -->
    {#if !finished && !cancelled}
      <ul class="steps steps-vertical w-full text-sm [&>li]:px-2">
        <li class="step step-primary" data-content="✓">正在解析样式</li>
        <li class="step {stage === '正在检查字号' || stage === '正在生成报告' ? 'step-primary' : ''}"
            data-content={stage === '正在检查字号' ? '●' : stage === '正在生成报告' ? '✓' : ''}>
          正在检查字号
        </li>
        <li class="step {stage === '正在生成报告' ? 'step-primary' : ''}"
            data-content={stage === '正在生成报告' ? '●' : ''}>
          正在生成报告
        </li>
      </ul>
      {#if stage}
        <p class="text-sm text-base-content/70">{stage}，请稍候…</p>
      {/if}
    {:else if finished}
      <div role="status" class="alert alert-success text-sm py-2">
        <span>审核完成，结果已在下方展示。</span>
      </div>
    {:else if cancelled}
      <div role="status" class="alert alert-warning text-sm py-2">
        <span>审核已停止，已完成部分的结果仍然可查看。</span>
      </div>
    {/if}

    <!-- 取消按钮：仅运行中显示 -->
    {#if !finished && !cancelled}
      <button type="button" class="btn btn-error w-full" onclick={onCancel}>
        取消审核
      </button>
    {/if}
  </div>
</section>
