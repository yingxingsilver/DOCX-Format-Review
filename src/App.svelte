<!--
  App.svelte —— 骨架页面。
  仅负责展示与调用 Tauri 命令，不包含任何审核业务逻辑。
-->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import ReportExport from "./ReportExport.svelte";

  let coreVer = $state("检测中…");

  $effect(() => {
    invoke<string>("core_version")
      .then((v) => (coreVer = v))
      .catch(() => (coreVer = "未连接"));
  });
</script>

<main class="min-h-screen bg-base-200 flex flex-col items-center justify-center gap-6 p-8">
  <div class="text-center">
    <h1 class="text-2xl font-bold">论文格式审核系统准备就绪</h1>
    <p class="opacity-70 mt-2">批量 DOCX 格式检查 · 本地运行 · 无需联网 · 不使用 AI</p>
    <p class="text-sm text-primary mt-4">核心引擎版本：{coreVer}</p>
  </div>

  <!-- P6：报告导出面板。审核结果将由批量检查阶段（P5 UI）接入后传入。 -->
  <div class="w-full max-w-md">
    <ReportExport />
  </div>
</main>

