<!--
  App.svelte —— P8B/P8C 主布局组合根。
  Svelte 5 runes 风格（项目 svelte ^5.57）。

  职责：
  - 组合 Header + 左栏任务配置（FilePicker / RuleSelector / ActionPanel）
    + 右栏工作区（WorkspacePanel；running/done 时接入 ProgressPanel / ResultTable /
      ViolationDetail / LogPanel，见 P8C）；
  - 从状态层读取全局状态，把动作以回调形式下发给子组件；
  - 不直接调用 Tauri invoke，不含 DOCX 审核业务逻辑。

  状态来源说明：当前使用 src/lib/uiState.ts（可本地模拟的状态层），
  其字段与动作命名和 src/lib/stores.ts（P8A 后端接线层）一一对应；
  接入真实后端时把下方 import 换成 "./lib/stores" 并关闭 simulateLocally 即可。
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
    currentFileName,
    stageLabel,
    report,
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
    selectFile,
  } from "./lib/uiState";

  /** 未选择文件夹时禁用「开始批量审核」。 */
  const canStart = $derived($inputDir !== null);
  /** done/cancelled/error 或已有摘要时允许清空结果。 */
  const hasResults = $derived(
    $status === "done" || $status === "cancelled" || $status === "error" || $summaryCounts !== null,
  );
  /** P8C：结果表数据源（报告已就绪时）。 */
  const resultFiles = $derived($report?.files ?? []);
  /** P8C：当前选中文件的完整结果（含 parse_error / 计数）。 */
  const selectedSummary = $derived(
    $selectedFile ? ($report?.files.find((f) => f.file_path === $selectedFile) ?? null) : null,
  );

  /** 拖拽回调占位：后续在此接 Tauri 窗口级 drag-drop 的真实路径。 */
  function handleDropPath(_path: string): void {
    // TODO(P9): 接入 Tauri onDragDropEvent，拿到真实目录后写入 inputDir 并刷新计数。
  }
</script>

<div data-theme="light" class="min-h-screen bg-base-200 text-base-content min-w-[1024px] lg:min-w-0">
  <!-- 顶部 Header -->
  <header class="bg-base-100 border-b border-base-200">
    <div class="mx-auto max-w-screen-2xl px-4 py-3 lg:px-6">
      <h1 class="text-xl font-bold text-base-content leading-tight">论文格式批量审核系统</h1>
      <p class="text-sm text-base-content/60 pt-0.5">
        离线检查 DOCX 字号、字体、行距、标题、标点等格式问题
      </p>
    </div>
  </header>

  <!-- 主体两栏：≥1024px 左右分栏，更窄时上下堆叠 -->
  <main class="mx-auto max-w-screen-2xl p-4 lg:p-6 grid gap-4 lg:gap-6 grid-cols-1 lg:grid-cols-[380px_minmax(0,1fr)]">
    <!-- 左栏：任务配置区 -->
    <div class="flex flex-col gap-4 lg:gap-6">
      <FilePicker
        inputDir={$inputDir}
        status={$status}
        fileCount={$fileCount}
        onChoose={chooseInputDir}
        onDropPath={handleDropPath}
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
        exportsReady={false}
        onIncludeSubdirsChange={(v) => includeSubdirs.set(v)}
        onGenerateAnnotatedChange={(v) => generateAnnotatedDocx.set(v)}
        onStart={startAudit}
        onCancel={cancelAudit}
        onClear={clearResults}
        onExportHtml={exportHtml}
        onExportCsv={exportCsv}
      />
    </div>

    <!-- 右栏：状态与结果工作区 -->
    <div class="flex flex-col gap-4 lg:gap-6 min-w-0">
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
