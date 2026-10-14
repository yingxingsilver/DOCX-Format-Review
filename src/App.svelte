<!--
  App.svelte —— P8B/P8C 主布局组合根（极简留白风格）。
  Svelte 5 runes 风格（项目 svelte ^5.57）。

  职责：
  - 组合 Header + 左栏任务配置（FilePicker / RuleSelector / ActionPanel）
    + 右栏工作区（WorkspacePanel；running/done 时接入 ProgressPanel / ResultTable /
      ViolationDetail / LogPanel，见 P8C）；
  - 从状态层读取全局状态，把动作以回调形式下发给子组件；
  - 不直接调用 Tauri invoke，不含 DOCX 审核业务逻辑。

  状态来源：src/lib/stores.ts（P8A/P8C 真实后端接线层）——进度、日志、结果
  全部来自 audit://* 事件；本地模拟层 uiState.ts 已移除。
-->
<script lang="ts">
  import FilePicker from "./components/FilePicker.svelte";
  import RuleSelector from "./components/RuleSelector.svelte";
  import ActionPanel from "./components/ActionPanel.svelte";
  import WorkspacePanel from "./components/WorkspacePanel.svelte";
  import ProgressPanel from "./components/ProgressPanel.svelte";
  import ResultTable from "./components/ResultTable.svelte";
  import ViolationDetail from "./components/ViolationDetail.svelte";
  import LogPanel from "./components/LogPanel.svelte";
  import {
    status,
    inputDir,
    ruleFileName,
    includeSubdirs,
    generateAnnotatedDocx,
    fileCount,
    errorMessage,
    progress,
    summaryCounts,
    summary,
    currentFileName,
    stageLabel,
    selectedFile,
    logs,
    chooseInputDir,
    chooseRuleFile,
    resetRule,
    startAudit,
    cancelAudit,
    clearResults,
    exportHtml,
    exportCsv,
    openAnnotatedFolder,
    selectFile,
    draggingOver,
    initDragDrop,
  } from "./lib/stores";

  /** 未选择文件夹时禁用「开始批量审核」（stores.startAudit 内还会校验规则文件）。 */
  const canStart = $derived($inputDir !== null);
  /** done/cancelled/error 或已有摘要时允许清空结果。 */
  const hasResults = $derived(
    $status === "done" || $status === "cancelled" || $status === "error" || $summaryCounts !== null,
  );
  /** P8C：结果表数据源（报告已就绪时）。 */
  const resultFiles = $derived($summary?.files ?? []);
  /** P8C：当前选中文件的完整结果（含 parse_error / 计数）。 */
  const selectedSummary = $derived(
    $selectedFile ? ($summary?.files.find((f) => f.file_path === $selectedFile) ?? null) : null,
  );

  /**
   * 接线 Tauri 窗口级 drag-drop：enter/leave 驱动拖拽高亮，drop 拿到真实路径后
   * 由 stores.handleDroppedPaths 设为论文文件夹 / 规则文件（见 stores.ts）。
   */
  $effect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void initDragDrop().then((u) => {
      if (disposed) u?.();
      else unlisten = u;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });
</script>

<div
  data-theme="light"
  class="min-h-screen bg-white text-ink min-w-[1024px] lg:min-w-0 transition-colors"
  class:ring-2={$draggingOver}
  class:ring-accent={$draggingOver}
>
  <!-- 全局拖拽遮罩：原生文件被拖入窗口时提示可直接放置 -->
  {#if $draggingOver}
    <div
      class="pointer-events-none fixed inset-0 z-50 flex items-center justify-center bg-accent/5"
      role="status"
      aria-label="松开鼠标即可将文件夹设为论文文件夹"
    >
      <div class="rounded-card border border-dashed border-accent bg-white px-6 py-4 text-sm text-ink shadow">
        松开鼠标：文件夹将作为论文文件夹，.json 将作为规则文件
      </div>
    </div>
  {/if}

  <!-- 顶部 Header：细线分隔，无阴影 -->
  <header class="border-b border-line bg-white">
    <div class="mx-auto max-w-screen-xl px-6 py-5 lg:px-10 flex items-baseline justify-between gap-4">
      <h1 class="font-heading text-[22px] font-semibold leading-none text-ink">论文格式批量审核</h1>
      <p class="hidden sm:block text-sm text-subtle">离线检查 DOCX 的字号、字体、行距、标题与标点</p>
    </div>
  </header>

  <!-- 主体两栏：≥1024px 左右分栏，更窄时上下堆叠；留白优先 -->
  <main class="mx-auto max-w-screen-xl px-6 py-8 lg:px-10 lg:py-10 grid gap-6 lg:gap-10 grid-cols-1 lg:grid-cols-[360px_minmax(0,1fr)]">
    <!-- 左栏：任务配置区 -->
    <div class="flex flex-col gap-5 lg:gap-6">
      <FilePicker
        inputDir={$inputDir}
        status={$status}
        fileCount={$fileCount}
        onChoose={chooseInputDir}
        dragging={$draggingOver}
      />
      <RuleSelector
        ruleFileName={$ruleFileName}
        status={$status}
        onChoose={chooseRuleFile}
        onReset={resetRule}
      />
      <ActionPanel
        status={$status}
        canStart={canStart}
        includeSubdirs={$includeSubdirs}
        generateAnnotatedDocx={$generateAnnotatedDocx}
        hasResults={hasResults}
        errorMessage={$errorMessage}
        exportsReady={$summary !== null}
        onIncludeSubdirsChange={(v) => includeSubdirs.set(v)}
        onGenerateAnnotatedChange={(v) => generateAnnotatedDocx.set(v)}
        onStart={startAudit}
        onCancel={cancelAudit}
        onClear={clearResults}
        onExportHtml={exportHtml}
        onExportCsv={exportCsv}
        onOpenAnnotatedFolder={openAnnotatedFolder}
      />
    </div>

    <!-- 右栏：状态与结果工作区 -->
    <div class="flex flex-col gap-5 lg:gap-6 min-w-0">
      {#if $status === "running"}
        <!-- P8C：进度面板（取代 P8B 的 running 占位） -->
        <ProgressPanel
          progress={$progress}
          currentFileName={$currentFileName}
          stage={$stageLabel}
          onCancel={cancelAudit}
        />
      {:else if $status === "done" || $status === "cancelled"}
        <!-- P8C：完成/取消 → 摘要统计 + 结果表 + 违规详情 + 运行详情 -->
        <WorkspacePanel
          status={$status}
          progress={$progress}
          currentFileName={$currentFileName}
          summary={$summaryCounts}
          errorMessage={$errorMessage}
          onRestart={clearResults}
        />
        {#if resultFiles.length > 0}
          <ResultTable
            files={resultFiles}
            selected={$selectedFile}
            onSelectFile={selectFile}
          />
          <ViolationDetail
            file={selectedSummary}
            onClose={() => selectFile(null)}
          />
        {/if}
        <LogPanel logs={$logs} />
      {:else}
        <!-- idle / error：沿用 P8B 工作区（欢迎 / 错误提示） -->
        <WorkspacePanel
          status={$status}
          progress={$progress}
          currentFileName={$currentFileName}
          summary={$summaryCounts}
          errorMessage={$errorMessage}
          onRestart={clearResults}
        />
        {#if $status === "error" && $logs.length > 0}
          <LogPanel logs={$logs} />
        {/if}
      {/if}
    </div>
  </main>
</div>

<style>
  .bg-white {
    background-color: var(--bg);
  }
  .text-ink {
    color: var(--ink);
  }
  .text-subtle {
    color: var(--muted);
  }
  .border-line {
    border-color: var(--line);
  }
  .font-heading {
    font-family: var(--font-heading);
  }
</style>
