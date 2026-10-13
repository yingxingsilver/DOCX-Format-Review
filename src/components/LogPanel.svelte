<!--
  LogPanel.svelte —— 运行详情（可选）折叠面板（P8C）。
  默认折叠，面向高级用户排查；普通用户不会看到技术内容。
  数据来自 props（App 从 stores.logs / uiState.logs 下发），组件内无业务逻辑。
-->
<script lang="ts">
  /** 与状态层 UiLogLine / 后端 LogEvent 兼容的展示行。 */
  import type { UiLogLine } from "../lib/uiState";
  import { ICONS } from "../lib/icons";

  interface Props {
    logs: UiLogLine[];
  }

  let { logs }: Props = $props();

  const errorCount = $derived(logs.filter((l) => l.level === "error").length);

  function timeOf(at: number): string {
    const d = new Date(at);
    const p = (n: number, w = 2) => String(n).padStart(w, "0");
    return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  }
</script>

{#if logs.length > 0}
  <details class="bg-white border border-line rounded-card">
    <summary class="cursor-pointer select-none px-5 py-3.5 text-sm text-subtle hover:text-ink flex items-center gap-2">
      <span class="icon" style="--icon-size:14px" aria-hidden="true">{@html ICONS.chevronDown}</span>
      运行详情
      <span class="tabular-nums">（{logs.length} 条{errorCount > 0 ? `，${errorCount} 条异常` : ""}）</span>
    </summary>
    <div class="border-t border-line px-5 py-4 max-h-56 overflow-y-auto">
      <ul class="flex flex-col gap-1.5">
        {#each logs as line, i (i)}
          <li class="flex items-start gap-3 text-xs leading-relaxed font-mono">
            <span class="tabular-nums text-subtle shrink-0">{timeOf(line.at)}</span>
            <span class={"shrink-0 " + (line.level === "error" ? "text-danger" : line.level === "warn" ? "text-warn" : "text-subtle")}>
              {levelText(line.level)}
            </span>
            <span class="text-ink-2 break-all">{line.message}</span>
          </li>
        {/each}
      </ul>
      <p class="pt-3 text-xs text-subtle">这些记录供熟悉电脑的老师排查使用，平时无需关注。</p>
    </div>
  </details>
{/if}

<style>
  .bg-white { background-color: var(--bg); }
  .rounded-card { border-radius: var(--radius-card); }
  .border-line { border-color: var(--line); }
  .text-ink { color: var(--ink); }
  .text-ink-2 { color: var(--ink-2); }
  .text-subtle { color: var(--muted); }
  .text-danger { color: #b42318; }
  .text-warn { color: #b45309; }
  details[open] summary .icon { transform: rotate(0deg); }
  details:not([open]) summary .icon { transform: rotate(-90deg); }
</style>
