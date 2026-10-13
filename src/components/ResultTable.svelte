<!--
  ResultTable.svelte —— 审核结果表（P8C）。
  列：文件名 / 状态 / 错误数 / 警告数 / 首个问题页 / 操作（查看详情）。
  支持按文件名搜索、按状态筛选；点击行触发 onSelectFile，由父级展示 ViolationDetail。
  数据来自 props（App 从 stores/uiState 的 report 下发），组件内无业务逻辑、不调用 Tauri。
-->
<script lang="ts">
  import type { AuditSummary } from "../lib/types";

  interface Props {
    /** 全部文件结果（stores: summary.files）。 */
    files: AuditSummary[];
    /** 当前选中文件路径（受控高亮）。 */
    selected?: string | null;
    onSelectFile: (path: string) => void;
  }

  let { files, selected = null, onSelectFile }: Props = $props();

  type FilterKey = "all" | "problem" | "read_failed";

  let keyword = $state("");
  let filter = $state<FilterKey>("all");

  const filtered = $derived.by(() => {
    const kw = keyword.trim().toLowerCase();
    return files.filter((f) => {
      if (kw && !f.file_name.toLowerCase().includes(kw)) return false;
      if (filter === "problem")
        return f.status === "needs_revision" || f.error_count + f.warning_count > 0;
      if (filter === "read_failed") return f.status === "read_failed";
      return true;
    });
  });

  /** 状态 → 中文与徽章配色（daisyUI 语义类，禁止硬编码颜色）。 */
  function statusBadge(s: AuditSummary): { text: string; cls: string } {
    switch (s.status) {
      case "passed":
        return { text: "通过", cls: "badge badge-success badge-sm" };
      case "needs_revision":
        return { text: "需修改", cls: "badge badge-warning badge-sm" };
      case "read_failed":
        return { text: "读取失败", cls: "badge badge-error badge-sm" };
      default:
        return { text: "已跳过", cls: "badge badge-ghost badge-sm" };
    }
  }

  /** 首个问题页（page_hint 最小值）；无则显示 —。 */
  function firstProblemPage(s: AuditSummary): string {
    const pages = s.violations
      .map((v) => v.location.page_hint)
      .filter((p): p is number => p != null);
    return pages.length > 0 ? `约第 ${Math.min(...pages)} 页` : "—";
  }

  /**
   * 行唯一键：优先 file_path；若报告内出现重复路径（如后端按不同规则输出同路径
   * 条目），追加下标兜底，避免 Svelte keyed each 因重复键直接抛错（审查项 2）。
   * O(1)：用 Map 统计已见次数，替代 indexOf 的 O(n²)。
   */
  const seenCount = $derived.by(() => {
    const m = new Map<string, number>();
    for (const f of files) m.set(f.file_path, (m.get(f.file_path) ?? 0) + 1);
    return m;
  });
  function rowKey(f: AuditSummary, i: number): string {
    return (seenCount.get(f.file_path) ?? 0) > 1 ? `${f.file_path}#${i}` : f.file_path;
  }
</script>

<section class="card border border-line bg-white">
  <div class="card-body gap-4">
    <div class="flex items-baseline justify-between gap-3 flex-wrap">
      <h2 class="font-heading text-[17px] font-semibold text-ink leading-none">检查结果</h2>
      <span class="text-sm text-subtle">{filtered.length} / {files.length} 篇</span>
    </div>

    <!-- 搜索 + 筛选 -->
    <div class="flex flex-wrap items-center gap-2">
      <label class="input input-bordered flex items-center gap-2 w-64" for="file-search">
        <span class="sr-only">按文件名搜索</span>
        <input
          id="file-search"
          type="search"
          class="grow"
          placeholder="按文件名查找"
          bind:value={keyword}
        />
      </label>
      <div class="join" role="group" aria-label="结果筛选">
        <button
          type="button"
          class="join-item btn btn-sm {filter === 'all' ? 'btn-active-token' : 'btn-outline'}"
          aria-pressed={filter === "all"}
          onclick={() => (filter = "all")}
        >全部</button>
        <button
          type="button"
          class="join-item btn btn-sm {filter === 'problem' ? 'btn-active-token' : 'btn-outline'}"
          aria-pressed={filter === "problem"}
          onclick={() => (filter = "problem")}
        >有问题</button>
        <button
          type="button"
          class="join-item btn btn-sm {filter === 'read_failed' ? 'btn-active-token' : 'btn-outline'}"
          aria-pressed={filter === "read_failed"}
          onclick={() => (filter = "read_failed")}
        >读取失败</button>
      </div>
    </div>

    <!-- 表格 -->
    <div class="overflow-x-auto max-h-[420px] overflow-y-auto rounded-card border border-line">
      <table class="table table-sm">
        <thead class="bg-surface text-ink [&_th]:sticky [&_th]:top-0 [&_th]:z-10">
          <tr>
            <th>文件名</th>
            <th>状态</th>
            <th class="text-right">错误</th>
            <th class="text-right">警告</th>
            <th>首个问题页</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#if filtered.length === 0}
            <tr>
              <td colspan="6" class="py-8 text-center text-subtle">
                {keyword ? "没有匹配的文件，换个关键词试试。" : "暂无结果。"}
              </td>
            </tr>
          {:else}
            {#each filtered as f, i (rowKey(f, i))}
              {@const badge = statusBadge(f)}
              <tr
                data-selected={selected === f.file_path ? "true" : "false"}
                onclick={() => onSelectFile(f.file_path)}
                onkeydown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onSelectFile(f.file_path);
                  }
                }}
                tabindex="0"
                aria-label={`查看 ${f.file_name} 的详情`}
              >
                <td class="max-w-[240px]">
                  <span class="block truncate text-ink-2" title={f.file_path}>{f.file_name}</span>
                </td>
                <td><span class={badge.cls}>{badge.text}</span></td>
                <td class="text-right tabular-nums text-ink-2">{f.error_count > 0 ? f.error_count : "—"}</td>
                <td class="text-right tabular-nums text-ink-2">{f.warning_count > 0 ? f.warning_count : "—"}</td>
                <td class="text-ink-2">{firstProblemPage(f)}</td>
                <td>
                  <button
                    type="button"
                    class="btn btn-ghost btn-xs"
                    onclick={(e) => {
                      e.stopPropagation();
                      onSelectFile(f.file_path);
                    }}
                  >查看详情</button>
                </td>
              </tr>
            {/each}
          {/if}
        </tbody>
      </table>
    </div>
  </div>
</section>

<style>
  .bg-white { background-color: var(--bg); }
  .bg-surface { background-color: var(--surface); }
  .text-ink { color: var(--ink); }
  .text-ink-2 { color: var(--ink-2); }
  .text-subtle { color: var(--muted); }
  .border-line { border-color: var(--line); }
  .rounded-card { border-radius: var(--radius-card); }
  .font-heading { font-family: var(--font-heading); }
  .sr-only {
    position: absolute;
    width: 1px; height: 1px;
    padding: 0; margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
  /* 筛选选中态：浅底 + 深色文字（强调色留给主 CTA） */
  .btn-active-token {
    background: var(--surface);
    border-color: var(--line);
    color: var(--ink);
  }
  tr[data-selected="true"] td {
    background: var(--surface);
  }
  tbody tr {
    cursor: pointer;
  }
</style>
