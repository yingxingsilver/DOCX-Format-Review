<!--
  App.svelte —— 骨架页面。
  仅负责展示与调用 Tauri 命令，不包含任何审核业务逻辑。
-->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  let coreVer = $state("检测中…");

  $effect(() => {
    invoke<string>("core_version")
      .then((v) => (coreVer = v))
      .catch(() => (coreVer = "未连接"));
  });
</script>

<main>
  <h1>论文格式审核系统准备就绪</h1>
  <p class="sub">批量 DOCX 格式检查 · 本地运行 · 无需联网 · 不使用 AI</p>
  <p class="status">核心引擎版本：{coreVer}</p>
</main>

<style>
  main {
    font-family: system-ui, "Microsoft YaHei", sans-serif;
    text-align: center;
    padding: 4rem 1rem;
    color: #1f2937;
  }
  h1 {
    font-size: 1.8rem;
  }
  .sub {
    color: #6b7280;
  }
  .status {
    margin-top: 2rem;
    font-size: 0.9rem;
    color: #2563eb;
  }
</style>
