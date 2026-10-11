<!--
  App.svelte —— P8D 主布局组合根（真实后端接线，完成 GUI 最后整合）。

  闭环：选择文件夹 → 选择规则 → 开始审核 → 查看结果 → 导出报告 → 打开报告/原文件。

  职责：
  - 组合全部组件：FilePicker / RuleSelector / ActionPanel / ProgressPanel /
    ResultTable / ViolationDetail / ExportBar / LogPanel（+ WorkspacePanel 摘要）；
  - 状态机渲染：idle（欢迎+配置）/ running（进度）/ done（结果+导出）/
    error（中文提示+重试）/ cancelled（已完成部分可查看+重新审核）；
  - 从状态层（stores.ts）读全局状态，把动作以回调下发给子组件；
  - 组件内不直接调用 Tauri invoke，不含 DOCX 审核业务逻辑；
  - 面向非技术用户的界面文案避免出现底层技术词汇，
    技术细节统一收纳进折叠的「运行详情（可选）」（LogPanel）。
-->
<script lang="ts">
  import { onDestroy } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import FilePicker from "./components/FilePicker.svelte";
  import RuleSelector from "./components/RuleSelector.svelte";
  import ActionPanel from "./components/ActionPanel.svelte";
  import WorkspacePanel from "./components/WorkspacePanel.svelte";
  import ProgressPanel from "./components/ProgressPanel.svelte";
  import ResultTable from "./components/ResultTable.svelte";
  import ViolationDetail from "./components/ViolationDetail.svelte";
  import ExportBar from "./components/ExportBar.svelte";
  import LogPanel from "./components/LogPanel.svelte";
  import type { ExportedFiles } from "./lib/types";
  import {
    status,
    inputDir,
    ruleFileName,
    fileCount,
    includeSubdirs,
    generateAnnotatedDocx,
    errorMessage,
    progress,
    currentFileName,
    stageLabel,
    summary,
    summaryCounts,
    selectedFile,
    logLines,
    exporting,
    exportedFiles,
    notice,
    chooseInputDir,
    chooseRuleFile,
    resetRule,
    startAudit,
    cancelAudit,
    clearResults,
    exportHtml,
    exportCsv,
    openExportedFile,
    openReportFolder,
    selectFile,
    disposeAuditEvents,
  } from "./lib/stores";

  /** 未选择文件夹时禁用「开始批量审核」；running 期间同样禁用（双重保险）。 */
  const canStart = $derived($inputDir !== null && $status !== "running");
  /** done/cancelled/error 或已有摘要时允许清空结果。 */
  const hasResults = $derived(
    $status === "done" || $status === "cancelled" || $status === "error" || $summaryCounts !== null,
  );
  /** 结果表数据源（报告已就绪时）。 */
  const resultFiles = $derived($summary?.files ?? []);
  /** 当前选中文件的完整结果（含 read_failed 原因 / 计数）。 */
  const selectedSummary = $derived(
    $selectedFile ? ($summary?.files.find((f) => f.file_path === $selectedFile) ?? null) : null,
  );
  /** 完成或取消后展示导出栏（取消时已完成部分同样可导出）。 */
  const showExport = $derived($status === "done" || $status === "cancelled");
  /** 勾选了标注版且存在需修改文件 → ExportBar 显示生成说明。 */
  const annotatedPromised = $derived(
    $generateAnnotatedDocx && ($summary?.failed_files ?? 0) > 0,
  );

  /** 拖拽回调占位：后续在此接 Tauri 窗口级 drag-drop 的真实路径。 */
  function handleDropPath(_path: string): void {
    // TODO(P9): 接入 Tauri onDragDropEvent，拿到真实目录后写入 inputDir 并刷新计数。
  }

  // =========================
  // 窗口关闭拦截（P8D 要求 3）
  // 运行中点关闭 → 中文确认「审核仍在进行，确定退出吗？」。
  // 选「仍要退出」时直接销毁窗口结束进程：审核线程随进程一起终止，
  // 不会留下无人认领的后台任务（无僵尸任务）。浏览器环境（npm run dev）自动跳过。
  // =========================
  let unlistenClose: (() => void) | null = null;
  let closingGuard = false;
  $effect(() => {
    if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
    getCurrentWindow()
      .onCloseRequested(async (event) => {
        if ($status !== "running" || closingGuard) return;
        event.preventDefault();
        closingGuard = true;
        try {
          const ok = await confirm("审核仍在进行，确定退出吗？", {
            title: "论文格式审核系统",
            kind: "warning",
            okLabel: "仍要退出",
            cancelLabel: "继续审核",
          });
          if (ok) await getCurrentWindow().destroy();
        } catch {
          /* 对话框不可用时不阻塞用户退出 */
        } finally {
          closingGuard = false;
        }
      })
      .then((fn) => {
        unlistenClose = fn;
      })
      .catch(() => {
        /* 拦截注册失败不影响正常使用 */
      });
    return () => {
      unlistenClose?.();
      unlistenClose = null;
    };
  });

  // 前端卸载时释放事件监听（Tauri 桌面应用里通常随进程退出，这里兜底）。
  onDestroy(() => disposeAuditEvents());
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

  <!-- 全局提示条（导出成功路径 / 中文错误提示，点击「知道了」关闭） -->
  {#if $notice}
    <div class="mx-auto max-w-screen-2xl px-4 lg:px-6 pt-3">
      <div
        role="status"
        class="alert {$notice.kind === "success" ? "alert-success" : "alert-error"} text-sm py-2"
      >
        <span>{$notice.text}</span>
        <button type="button" class="btn btn-ghost btn-xs ml-auto" onclick={() => notice.set(null)}>
          知道了
        </button>
      </div>
    </div>
  {/if}

  <!-- 主体两栏：≥1024px 左右分栏，更窄时上下堆叠 -->
  <main class="mx-auto max-w-screen-2xl p-4 lg:p-6 grid gap-4 lg:gap-6 grid-cols-1 lg:grid-cols-[380px_minmax(0,1fr)]">
    <!-- 左栏：任务配置区（running 时各子组件自行禁用） -->
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
        exportsReady={showExport}
        onIncludeSubdirsChange={(v) => includeSubdirs.set(v)}
        onGenerateAnnotatedChange={(v) => generateAnnotatedDocx.set(v)}
        onStart={startAudit}
        onCancel={cancelAudit}
        onClear={clearResults}
        onExportHtml={exportHtml}
        onExportCsv={exportCsv}
      />
    </div>

    <!-- 右栏：状态机工作区 -->
    <div class="flex flex-col gap-4 lg:gap-6 min-w-0">
      {#if $status === "idle"}
        <!-- P8D：首次使用说明（面向非技术人员，无技术词汇） -->
        <div role="note" class="alert alert-info text-sm">
          <span>将论文文件夹拖入或点击选择，系统会离线检查格式。全程不联网，也不会改动你的原文件。</span>
        </div>
        <WorkspacePanel
          status="idle"
          progress={$progress}
          summary={null}
          errorMessage={null}
          onRestart={clearResults}
        />
      {:else if $status === "running"}
        <!-- running：进度 + 当前文件 + 阶段文案；开始按钮已由 ActionPanel 禁用 -->
        <ProgressPanel
          progress={$progress}
          currentFileName={$currentFileName}
          stage={$stageLabel}
          onCancel={cancelAudit}
        />
      {:else if $status === "done" || $status === "cancelled"}
        <!-- done / cancelled：摘要统计 + 结果表 + 违规详情 + 导出栏 + 运行详情 -->
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
        <ExportBar
          enabled={showExport && $summary !== null}
          busy={$exporting}
          annotatedPromised={annotatedPromised}
          produced={$exportedFiles}
          onExportHtml={exportHtml}
          onExportCsv={exportCsv}
          onOpenFolder={openReportFolder}
          onOpenFile={(entry: ExportedFiles) => openExportedFile(entry)}
        />
        <LogPanel logs={$logLines} />
      {:else if $status === "error"}
        <!-- error：中文错误提示 + 「重新开始」（复用当前文件夹/规则配置） -->
        <WorkspacePanel
          status="error"
          progress={$progress}
          currentFileName={$currentFileName}
          summary={$summaryCounts}
          errorMessage={$errorMessage}
          onRestart={startAudit}
        />
        {#if $logLines.length > 0}
          <LogPanel logs={$logLines} />
        {/if}
      {/if}
    </div>
  </main>
</div>
