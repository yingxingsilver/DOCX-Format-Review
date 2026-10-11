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
</script>

<section class="card bg-base-100 border border-base-200 shadow-sm">
  <div class="card-body gap-3">
    <h2 class="card-title text-lg font-semibold text-base-content">检查结果</h2>

    <!-- 搜索 + 筛选 -->
    <div class="flex flex-wrap items-center gap-2">
      <label class="input input-bordered flex items-center gap-2 w-64" for="file-search">
        <span class="text-base-content/60 shrink-0">搜索</span>
        <input
          id="file-search"
          type="search"
          class="grow"
          placeholder="按文件名查找…"
          bind:value={keyword}
        />
      </label>
      <div class="join" role="group" aria-label="结果筛选">
        <button
          type="button"
          class="join-item btn btn-sm {filter === 'all' ? 'btn-primary' : 'btn-outline'}"
          aria-pressed={filter === "all"}
          onclick={() => (filter = "all")}
        >全部</button>
        <button
          type="button"
          class="join-item btn btn-sm {filter === 'problem' ? 'btn-primary' : 'btn-outline'}"
          aria-pressed={filter === "problem"}
          onclick={() => (filter = "problem")}
        >仅看有问题</button>
        <button
          type="button"
          class="join-item btn btn-sm {filter === 'read_failed' ? 'btn-primary' : 'btn-outline'}"
          aria-pressed={filter === "read_failed"}
          onclick={() => (filter = "read_failed")}
        >仅看读取失败</button>
      </div>
      <span class="text-sm text-base-content/60 ml-auto">共 {filtered.length} 条</span>
    </div>

    <!-- 表格 -->
    <div class="overflow-x-auto max-h-[420px] overflow-y-auto rounded-lg border border-base-200">
      <table class="table table-sm table-zebra">
        <thead class="bg-base-200 text-base-content [&_th]:sticky [&_th]:top-0 [&_th]:z-10">
          <tr>
            <th>文件名</th>
            <th>状态</th>
            <th class="text-right">错误数</th>
            <th class="text-right">警告数</th>
            <th>首个问题页</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          {#if filtered.length === 0}
            <tr>
              <td colspan="6" class="py-8 text-center text-base-content/60">
                {keyword ? "没有匹配的文件，换个关键词试试。" : "暂无结果。"}
              </td>
            </tr>
          {:else}
            {#each filtered as f (f.file_path)}
              {@const badge = statusBadge(f)}
              <tr
                class="cursor-pointer hover:bg-base-200 {selected === f.file_path ? '!bg-base-200' : ''}"
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
                  <span class="block truncate" title={f.file_path}>{f.file_name}</span>
                </td>
                <td><span class={badge.cls}>{badge.text}</span></td>
                <td class="text-right tabular-nums">{f.error_count > 0 ? f.error_count : "—"}</td>
                <td class="text-right tabular-nums">{f.warning_count > 0 ? f.warning_count : "—"}</td>
                <td>{firstProblemPage(f)}</td>
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

    <p class="text-sm text-base-content/60">点击任意一行可在下方查看该论文的具体问题。</p>
  </div>
</section>
