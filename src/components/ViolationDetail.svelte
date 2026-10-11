<!--
  ViolationDetail.svelte —— 单文件违规详情（P8C）。
  显示：文件名、按严重级别筛选的违规卡片列表（级别徽章/页码估算/段落序号/
  问题说明/期望格式/实际格式/原文摘录高亮/修改建议）、复制单条问题。
  数据来自 props（App 从 stores.violationsByFile + selectedFile 下发），无业务逻辑。
-->
<script lang="ts">
  import { onDestroy } from "svelte";
  import type { AuditSummary, Severity, Violation } from "../lib/types";

  interface Props {
    /** 当前选中的文件结果；null 表示未选择。 */
    file: AuditSummary | null;
    onClose?: () => void;
  }

  let { file, onClose = () => {} }: Props = $props();

  type SevFilter = "all" | Severity;
  let sevFilter = $state<SevFilter>("all");

  const filtered = $derived(
    file ? file.violations.filter((v) => sevFilter === "all" || v.severity === sevFilter) : [],
  );

  function sevBadge(s: Severity): { text: string; cls: string } {
    switch (s) {
      case "error":
        return { text: "错误", cls: "badge badge-error badge-sm" };
      case "warning":
        return { text: "警告", cls: "badge badge-warning badge-sm" };
      default:
        return { text: "提示", cls: "badge badge-info badge-sm" };
    }
  }

  /** 位置人话：大约第 N 页 · 第 M 段（与 stores.locationLabel 同语义，纯展示函数）。 */
  function locLabel(v: Violation): string {
    const parts: string[] = [];
    if (v.location.page_hint != null) parts.push(`大约第 ${v.location.page_hint} 页`);
    if (v.location.paragraph_index != null) parts.push(`第 ${v.location.paragraph_index + 1} 段`);
    return parts.length > 0 ? parts.join(" · ") : "整篇文档";
  }

  // ---- 复制单条问题 ----
  let copiedId = $state<string | null>(null);

  /**
   * “已复制”提示的自动消失定时器。
   * 用显式 timer + 销毁清理（onDestroy），而不是 $effect：
   * $effect 会在 copiedId 每次变化时重跑，用户 2 秒内复制另一条会把计时清零，
   * 导致“已复制”提示无限续期、永不复位；且同一问题连续复制时 id 不变、
   * effect 不重跑，行为不一致。审查项 11：定时器必须随组件销毁释放。
   */
  let copyResetTimer: ReturnType<typeof setTimeout> | null = null;

  function armCopyReset(id: string): void {
    if (copyResetTimer !== null) clearTimeout(copyResetTimer);
    copiedId = id;
    copyResetTimer = setTimeout(() => {
      copiedId = null;
      copyResetTimer = null;
    }, 2000);
  }

  onDestroy(() => {
    if (copyResetTimer !== null) {
      clearTimeout(copyResetTimer);
      copyResetTimer = null;
    }
  });

  /** 复制到剪贴板：优先 navigator.clipboard，失败回退隐藏 textarea + execCommand。 */
  async function copyText(text: string): Promise<boolean> {
    try {
      if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(text);
        return true;
      }
    } catch {
      /* 继续走回退方案 */
    }
    try {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.className = "fixed opacity-0";
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand("copy");
      document.body.removeChild(ta);
      return ok;
    } catch {
      return false;
    }
  }

  function formatViolation(v: Violation): string {
    const lines = [
      `[${sevBadge(v.severity).text}] ${v.message}`,
      `位置：${locLabel(v)}`,
    ];
    if (v.expected) lines.push(`期望格式：${v.expected}`);
    if (v.actual) lines.push(`实际格式：${v.actual}`);
    if (v.excerpt) lines.push(`原文摘录：${v.excerpt}`);
    if (v.suggestion) lines.push(`修改建议：${v.suggestion}`);
    return lines.join("\n");
  }

  async function onCopy(v: Violation): Promise<void> {
    const ok = await copyText(formatViolation(v));
    if (ok) armCopyReset(vidKey(v));
  }

  function vidKey(v: Violation): string {
    return `${v.rule_id}-${v.location.paragraph_index ?? -1}-${v.severity}`;
  }

  /** 原文摘录高亮片段索引（[start,end)）；无法定位时返回 null。 */
  function highlightRange(v: Violation): [number, number] | null {
    if (!v.excerpt || !v.actual) return null;
    const start = v.excerpt.indexOf(v.actual);
    if (start >= 0) return [start, start + v.actual.length];
    // actual 不在摘录中：无法可靠定位问题片段，不做示意性高亮，避免误导用户。
    return null;
  }
</script>

{#if file === null}
  <div class="card bg-base-100 border border-base-200 shadow-sm">
    <div class="card-body items-center justify-center py-10 text-center">
      <p class="text-base-content/60 text-sm">在上方结果表中点击一篇论文，即可在这里查看它的具体问题和修改建议。</p>
    </div>
  </div>
{:else}
  <section class="card bg-base-100 border border-base-200 shadow-sm">
    <div class="card-body gap-3">
      <!-- 头部：文件名 + 关闭 -->
      <div class="flex items-start justify-between gap-2">
        <div class="min-w-0">
          <h2 class="card-title text-lg font-semibold text-base-content truncate" title={file.file_path}>
            {file.file_name}
          </h2>
          <p class="text-sm text-base-content/60 pt-0.5">
            {#if file.status === "read_failed"}
              该文件未能读取
            {:else if file.violations.length === 0}
              未发现格式问题
            {:else}
              共发现 {file.violations.length} 个问题（错误 {file.error_count} · 警告 {file.warning_count} · 提示 {file.info_count}）
            {/if}
          </p>
        </div>
        <button type="button" class="btn btn-ghost btn-sm shrink-0" onclick={onClose}>收起详情</button>
      </div>

      {#if file.status === "read_failed"}
        <!-- 读取失败：友好原因，不显示堆栈 -->
        <div role="alert" class="alert alert-error text-sm">
          <div>
            <span class="font-semibold">这个文件没能打开</span>
            <p class="pt-1">{file.parse_error ?? "文件可能已损坏或被加密，请尝试另存为新文件后重新检查。"}</p>
          </div>
        </div>
      {:else if file.violations.length === 0}
        <div role="status" class="alert alert-success text-sm">
          <span>这篇论文未发现格式问题，可以放心提交。</span>
        </div>
      {:else}
        <!-- 严重级别筛选 -->
        <div class="join" role="group" aria-label="按严重级别筛选">
          <button type="button" class="join-item btn btn-sm {sevFilter === 'all' ? 'btn-primary' : 'btn-outline'}"
                  aria-pressed={sevFilter === "all"} onclick={() => (sevFilter = "all")}>
            全部（{file.violations.length}）
          </button>
          <button type="button" class="join-item btn btn-sm {sevFilter === 'error' ? 'btn-error' : 'btn-outline'}"
                  aria-pressed={sevFilter === "error"} onclick={() => (sevFilter = "error")}>
            错误（{file.error_count}）
          </button>
          <button type="button" class="join-item btn btn-sm {sevFilter === 'warning' ? 'btn-warning' : 'btn-outline'}"
                  aria-pressed={sevFilter === "warning"} onclick={() => (sevFilter = "warning")}>
            警告（{file.warning_count}）
          </button>
          <button type="button" class="join-item btn btn-sm {sevFilter === 'info' ? 'btn-info' : 'btn-outline'}"
                  aria-pressed={sevFilter === "info"} onclick={() => (sevFilter = "info")}>
            提示（{file.info_count}）
          </button>
        </div>

        {#if filtered.length === 0}
          <p class="text-sm text-base-content/60 py-4 text-center">该级别下没有问题。</p>
        {:else}
          <ul class="flex flex-col gap-3 max-h-[460px] overflow-y-auto pr-1">
            {#each filtered as v (vidKey(v))}
              {@const badge = sevBadge(v.severity)}
              {@const range = highlightRange(v)}
              <li class="rounded-xl border border-base-200 bg-base-100 p-4 shadow-sm">
                <div class="flex items-center gap-2 flex-wrap">
                  <span class={badge.cls}>{badge.text}</span>
                  <span class="text-sm text-base-content/70">{locLabel(v)}</span>
                  {#if v.requires_manual_review}
                    <span class="badge badge-ghost badge-sm">建议人工复核</span>
                  {/if}
                  <button type="button" class="btn btn-outline btn-xs ml-auto" onclick={() => onCopy(v)}>
                    {copiedId === vidKey(v) ? "已复制" : "复制此问题"}
                  </button>
                </div>

                <p class="pt-2 text-sm font-medium text-base-content">{v.message}</p>

                <dl class="pt-2 grid grid-cols-1 sm:grid-cols-2 gap-x-6 gap-y-1 text-sm">
                  {#if v.expected !== null}
                    <div class="flex gap-1"><dt class="shrink-0 text-base-content/60">期望格式：</dt><dd>{v.expected}</dd></div>
                  {/if}
                  {#if v.actual !== null}
                    <div class="flex gap-1"><dt class="shrink-0 text-base-content/60">实际格式：</dt><dd>{v.actual}</dd></div>
                  {/if}
                </dl>

                {#if v.excerpt !== null}
                  <div class="pt-2">
                    <p class="text-sm text-base-content/60">原文摘录：</p>
                    <blockquote class="mt-1 rounded-lg border-l-4 border-base-300 bg-base-200 px-3 py-2 text-sm leading-relaxed">
                      {#if range}
                        {v.excerpt.slice(0, range[0])}<mark class="rounded px-0.5 bg-warning/40 text-base-content">{v.excerpt.slice(range[0], range[1])}</mark>{v.excerpt.slice(range[1])}
                      {:else}
                        {v.excerpt}
                      {/if}
                    </blockquote>
                  </div>
                {/if}

                {#if v.suggestion !== null}
                  <div class="alert alert-info mt-3 text-sm py-2">
                    <span><span class="font-semibold">怎么改：</span>{v.suggestion}</span>
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    </div>
  </section>
{/if}
