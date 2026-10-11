<!--
  WorkspacePanel.svelte —— 右栏状态与结果工作区（按 status 切换五态界面）。
  P8B 阶段只做布局与占位，不实现结果明细表；不调用 Tauri invoke。
-->
<script lang="ts">
  import type { AuditRunStatus } from "../lib/types";

  interface SummaryCounts {
    total: number;
    passed: number;
    needsRevision: number;
    readFailed: number;
  }

  interface Props {
    status: AuditRunStatus;
    /** 进度数据；null 时用 skeleton 占位。 */
    progress: { completed: number; total: number } | null;
    /** 当前正在处理的文件名占位（stores 暂无此字段前可为 null）。 */
    currentFileName?: string | null;
    summary: SummaryCounts | null;
    errorMessage: string | null;
    onRestart: () => void;
  }

  let {
    status,
    progress,
    currentFileName = null,
    summary,
    errorMessage,
    onRestart,
  }: Props = $props();

  const percent = $derived(
    progress && progress.total > 0
      ? Math.round((progress.completed / progress.total) * 100)
      : 0,
  );
</script>

<div class="min-h-[420px]" aria-live="polite">
  {#if status === "idle"}
    <!-- 欢迎 / 空状态 -->
    <section class="card bg-base-100 border border-base-200 shadow-sm">
      <div class="card-body gap-4">
        <h2 class="card-title text-lg font-semibold text-base-content">
          欢迎使用论文格式批量审核系统
        </h2>
        <div role="status" class="alert alert-info text-sm">
          <div>
            <p>请在左侧选择论文文件夹。</p>
            <p class="pt-1 text-base-content/70">
              系统会离线检查 DOCX 文件的字号、字体、行距、标题、标点等格式问题。
            </p>
          </div>
        </div>
        <div class="flex flex-wrap gap-2">
          <span class="badge badge-info badge-outline gap-1 py-3 px-3">批量处理</span>
          <span class="badge badge-success badge-outline gap-1 py-3 px-3">离线运行</span>
          <span class="badge badge-warning badge-outline gap-1 py-3 px-3">中文报告</span>
        </div>
      </div>
    </section>

  {:else if status === "running"}
    <!-- 审核进行中占位 -->
    <section class="card bg-base-100 border border-base-200 shadow-sm">
      <div class="card-body gap-4">
        <h2 class="card-title text-lg font-semibold text-base-content">正在审核</h2>

        <div class="form-control">
          <label class="label" for="audit-progress">
            <span class="label-text">
              已处理 {progress?.completed ?? "—"} / {progress?.total ?? "—"} 个文件
            </span>
            <span class="label-text">{percent}%</span>
          </label>
          <progress
            id="audit-progress"
            class="progress progress-primary w-full"
            value={percent}
            max="100"
          ></progress>
        </div>

        <dl class="grid grid-cols-2 gap-3 text-sm">
          <div>
            <dt class="text-base-content/60">当前文件</dt>
            <dd class="pt-1">
              {#if currentFileName}
                <span class="truncate block" title={currentFileName}>{currentFileName}</span>
              {:else}
                <span class="skeleton block h-4 w-1/3"></span>
              {/if}
            </dd>
          </div>
          <div>
            <dt class="text-base-content/60">剩余数量</dt>
            <dd class="pt-1">
              {#if progress && progress.total > 0}
                {Math.max(progress.total - progress.completed, 0)} 个文件
              {:else}
                <span class="skeleton block h-4 w-1/2"></span>
              {/if}
            </dd>
          </div>
        </dl>

        <ul class="space-y-2 text-sm text-base-content/70">
          <li>正在解析文档结构</li>
          <li>正在检查字体与字号</li>
          <li>正在生成审核结果</li>
        </ul>

        <div class="skeleton h-10 w-full rounded-lg"></div>
      </div>
    </section>

  {:else if status === "done"}
    <!-- 结果摘要占位 -->
    <section class="card bg-base-100 border border-base-200 shadow-sm">
      <div class="card-body gap-4">
        <h2 class="card-title text-lg font-semibold text-base-content">审核完成</h2>

        <div class="stats stats-vertical sm:stats-horizontal shadow border border-base-200 w-full">
          <div class="stat">
            <div class="stat-title">总文件数</div>
            <div class="stat-value text-2xl">
              {#if summary}{summary.total}{:else}<span class="skeleton h-6 w-12"></span>{/if}
            </div>
          </div>
          <div class="stat">
            <div class="stat-title">通过</div>
            <div class="stat-value text-2xl text-success">
              {#if summary}{summary.passed}{:else}<span class="skeleton h-6 w-12"></span>{/if}
            </div>
          </div>
          <div class="stat">
            <div class="stat-title">需修改</div>
            <div class="stat-value text-2xl text-warning">
              {#if summary}{summary.needsRevision}{:else}<span class="skeleton h-6 w-12"></span>{/if}
            </div>
          </div>
          <div class="stat">
            <div class="stat-title">读取失败</div>
            <div class="stat-value text-2xl text-error">
              {#if summary}{summary.readFailed}{:else}<span class="skeleton h-6 w-12"></span>{/if}
            </div>
          </div>
        </div>

        <div class="card bg-base-200 border border-base-200">
          <div class="card-body py-4">
            <p class="text-sm text-base-content/60">详细结果将在下一阶段接入。</p>
            <div class="skeleton h-10 w-full mt-2 rounded-lg"></div>
          </div>
        </div>

        <button type="button" class="btn btn-primary w-full" onclick={onRestart}>
          重新审核
        </button>
      </div>
    </section>

  {:else if status === "error"}
    <!-- 错误提示 -->
    <section class="card bg-base-100 border border-base-200 shadow-sm">
      <div class="card-body gap-4">
        <div role="alert" class="alert alert-error">
          <div>
            <h2 class="font-semibold">审核失败</h2>
            <p class="text-sm pt-1">{errorMessage ?? "检查未能完成，请重试。"}</p>
          </div>
        </div>
        <button type="button" class="btn btn-primary w-full" onclick={onRestart}>
          重新开始
        </button>
      </div>
    </section>

  {:else if status === "cancelled"}
    <!-- 已取消提示 -->
    <section class="card bg-base-100 border border-base-200 shadow-sm">
      <div class="card-body gap-4">
        <div role="alert" class="alert alert-warning">
          <div>
            <h2 class="font-semibold">审核已取消</h2>
            <p class="text-sm pt-1">当前任务已停止，你可以重新选择文件夹并开始审核。</p>
          </div>
        </div>
        <button type="button" class="btn btn-primary w-full" onclick={onRestart}>
          重新审核
        </button>
      </div>
    </section>
  {/if}
</div>
