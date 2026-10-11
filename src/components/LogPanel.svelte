<!--
  LogPanel.svelte —— 运行详情（可选）折叠面板（P8C）。
  默认折叠，面向高级用户排查；普通用户不会看到技术内容。
  数据来自 props（App 从 stores.logs / uiState.logs 下发），组件内无业务逻辑。
-->
<script lang="ts">
  /** 与状态层 UiLogLine / 后端 LogEvent 兼容的展示行。 */
  interface LogLine {
    at: number;
    level: "info" | "warn" | "error";
    message: string;
  }

  interface Props {
    logs: LogLine[];
  }

  let { logs }: Props = $props();

  const errorCount = $derived(logs.filter((l) => l.level === "error").length);

  function timeOf(at: number): string {
    const d = new Date(at);
    const p = (n: number, w = 2) => String(n).padStart(w, "0");
    return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  }

  function levelBadge(level: LogLine["level"]): string {
    switch (level) {
      case "error":
        return "badge badge-error badge-sm shrink-0";
      case "warn":
        return "badge badge-warning badge-sm shrink-0";
      default:
        return "badge badge-ghost badge-sm shrink-0";
    }
  }

  function levelText(level: LogLine["level"]): string {
    return level === "error" ? "异常" : level === "warn" ? "提醒" : "记录";
  }
</script>

{#if logs.length > 0}
  <details class="bg-base-100 border border-base-200 rounded-xl shadow-sm">
    <summary class="cursor-pointer select-none px-4 py-3 text-sm font-medium text-base-content/70 hover:text-base-content">
      运行详情（可选）
      <span class="badge badge-ghost badge-sm ml-2">{logs.length} 条</span>
      {#if errorCount > 0}
        <span class="badge badge-error badge-sm ml-1">{errorCount} 条异常</span>
      {/if}
    </summary>
    <div class="border-t border-base-200 px-4 py-3 max-h-56 overflow-y-auto">
      <ul class="flex flex-col gap-1.5">
        {#each logs as line, i (i)}
          <li class="flex items-start gap-2 text-xs leading-relaxed">
            <span class="tabular-nums text-base-content/50 shrink-0">{timeOf(line.at)}</span>
            <span class={levelBadge(line.level)}>{levelText(line.level)}</span>
            <span class="text-base-content/80 break-all">{line.message}</span>
          </li>
        {/each}
      </ul>
      <p class="pt-3 text-xs text-base-content/50">这些记录供熟悉电脑的老师排查使用，平时无需关注。</p>
    </div>
  </details>
{/if}
