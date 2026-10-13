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
    <!-- 欢迎 / 空状态：单列叙述，留白为主 -->
    <section class="card border border-line bg-white">
      <div class="card-body gap-6 py-10 px-8">
        <div class="max-w-[52ch]">
          <h2 class="font-heading text-[26px] font-semibold text-ink leading-tight">
            开始一次格式检查
          </h2>
          <p class="text-[17px] text-ink-2 pt-3" style="line-height:1.65">
            在左侧选择存放论文的文件夹，点击「开始批量审核」。系统会逐个读取 DOCX，
            对照规则检查字号、字体、行距、标题层级和标点，并给出每个问题的修改建议。
          </p>
          <p class="text-sm text-subtle pt-3">
            全程在本机完成，文件不会上传到任何外部服务。
          </p>
        </div>

        <ol class="flex flex-col gap-3 text-[15px] text-ink-2 max-w-[52ch]">
          <li class="flex gap-3">
            <span class="text-subtle tabular-nums shrink-0">1</span>
            <span>选择论文文件夹（可包含子文件夹）</span>
          </li>
          <li class="flex gap-3">
            <span class="text-subtle tabular-nums shrink-0">2</span>
            <span>如有学校专用要求，选择对应的规则文件</span>
          </li>
          <li class="flex gap-3">
            <span class="text-subtle tabular-nums shrink-0">3</span>
            <span>查看结果列表，点开任意一篇看具体问题</span>
          </li>
        </ol>
      </div>
    </section>

  {:else if status === "running"}
    <!-- 审核进行中（ProgressPanel 未接入时的兜底展示） -->
    <section class="card border border-line bg-white">
      <div class="card-body gap-5">
        <h2 class="font-heading text-[17px] font-semibold text-ink leading-none">正在审核</h2>

        <div>
          <div class="flex items-baseline justify-between mb-2">
            <span class="text-sm text-subtle">
              已处理 {progress?.completed ?? "—"} / {progress?.total ?? "—"} 个文件
            </span>
            <span class="text-sm text-ink tabular-nums">{percent}%</span>
          </div>
          <progress
            class="progress progress-primary w-full"
            value={percent}
            max="100"
            aria-label="审核进度"
          ></progress>
        </div>

        <dl class="grid grid-cols-2 gap-x-6 gap-y-2 text-sm">
          <div class="flex gap-2">
            <dt class="text-subtle shrink-0">当前文件</dt>
            <dd class="min-w-0">
              {#if currentFileName}
                <span class="truncate block text-ink-2" title={currentFileName}>{currentFileName}</span>
              {:else}
                <span class="skeleton block h-4 w-1/3"></span>
              {/if}
            </dd>
          </div>
          <div class="flex gap-2">
            <dt class="text-subtle shrink-0">剩余</dt>
            <dd class="text-ink-2">
              {#if progress && progress.total > 0}
                {Math.max(progress.total - progress.completed, 0)} 个文件
              {:else}
                <span class="skeleton block h-4 w-1/2"></span>
              {/if}
            </dd>
          </div>
        </dl>
      </div>
    </section>

  {:else if status === "done"}
    <!-- 结果摘要：四个数字一屏读完，弱装饰 -->
    <section class="card border border-line bg-white">
      <div class="card-body gap-6">
        <div class="flex items-baseline justify-between gap-4 flex-wrap">
          <h2 class="font-heading text-[20px] font-semibold text-ink leading-none">本次检查结果</h2>
          <span class="text-sm text-subtle">
            {#if summary}{summary.total} 篇论文已完成检查{:else}正在汇总…{/if}
          </span>
        </div>

        <dl class="grid grid-cols-2 lg:grid-cols-4 gap-x-6 gap-y-6">
          <div>
            <dt class="text-sm text-subtle">总文件数</dt>
            <dd class="font-heading text-[28px] leading-none text-ink pt-2 tabular-nums">
              {#if summary}{summary.total}{:else}<span class="skeleton inline-block h-7 w-12 align-middle"></span>{/if}
            </dd>
          </div>
          <div>
            <dt class="text-sm text-subtle">通过</dt>
            <dd class="font-heading text-[28px] leading-none text-accent pt-2 tabular-nums">
              {#if summary}{summary.passed}{:else}<span class="skeleton inline-block h-7 w-12 align-middle"></span>{/if}
            </dd>
          </div>
          <div>
            <dt class="text-sm text-subtle">需修改</dt>
            <dd class="font-heading text-[28px] leading-none text-ink pt-2 tabular-nums">
              {#if summary}{summary.needsRevision}{:else}<span class="skeleton inline-block h-7 w-12 align-middle"></span>{/if}
            </dd>
          </div>
          <div>
            <dt class="text-sm text-subtle">读取失败</dt>
            <dd class="font-heading text-[28px] leading-none text-danger pt-2 tabular-nums">
              {#if summary}{summary.readFailed}{:else}<span class="skeleton inline-block h-7 w-12 align-middle"></span>{/if}
            </dd>
          </div>
        </dl>

        <p class="text-sm text-subtle">在下方列表中点击文件名，查看该论文的具体问题与修改建议。</p>
      </div>
    </section>

  {:else if status === "error"}
    <!-- 错误提示：只说清原因和下一步 -->
    <section class="card border border-line bg-white">
      <div class="card-body gap-5">
        <h2 class="font-heading text-[20px] font-semibold text-ink leading-none">检查未能完成</h2>
        <p class="text-[15px] text-ink-2 max-w-[60ch]" style="line-height:1.65">
          {errorMessage ?? "审核过程中出现异常，请确认文件夹可访问后重试。"}
        </p>
        <div>
          <button type="button" class="btn btn-primary" onclick={onRestart}>
            重新开始
          </button>
        </div>
      </div>
    </section>

  {:else if status === "cancelled"}
    <!-- 已取消：保留部分结果入口 -->
    <section class="card border border-line bg-white">
      <div class="card-body gap-5">
        <h2 class="font-heading text-[20px] font-semibold text-ink leading-none">审核已停止</h2>
        <p class="text-[15px] text-ink-2 max-w-[60ch]" style="line-height:1.65">
          任务已中断。已完成部分的检查结果仍然可以查看；如需继续，重新发起即可。
        </p>
        <div>
          <button type="button" class="btn btn-outline" onclick={onRestart}>
            清空并重新开始
          </button>
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  .bg-white { background-color: var(--bg); }
  .text-ink { color: var(--ink); }
  .text-ink-2 { color: var(--ink-2); }
  .text-subtle { color: var(--muted); }
  .text-accent { color: var(--accent); }
  .text-danger { color: #b42318; }
  .border-line { border-color: var(--line); }
  .font-heading { font-family: var(--font-heading); }
</style>
