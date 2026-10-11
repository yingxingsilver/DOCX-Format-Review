<!--
  ReportExport.svelte —— P6 报告导出面板（daisyUI）。

  职责边界：
  - 只负责展示与调用 Tauri 命令（export_report），不做任何审核/渲染业务逻辑；
  - 审核结果数据（AuditReport JSON）由上游阶段（P5 批量审核 UI）通过 prop 传入。

  生命周期约定：本组件不手动 addEventListener——交互全部走 Svelte 的 on:click，
  异步回调写入 $state 前用 destroyed 标志位防护，避免销毁后 setState。
-->
<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  /** 上游传入的 AuditReport（serde_json 序列化后的对象；为 null 时按钮禁用）。 */
  let { reportJson = null }: { reportJson?: unknown | null } = $props();

  type Format = "html" | "csv";

  let format = $state<Format>("html");
  let busy = $state(false);
  let message = $state("");
  let isError = $state(false);

  // 组件销毁标志：防止在途 invoke 回调在销毁后继续写状态。
  let destroyed = false;
  $effect(() => () => (destroyed = true));

  async function doExport() {
    if (!reportJson) {
      message = "还没有审核结果，请先完成一次检查。";
      isError = true;
      return;
    }
    busy = true;
    message = "";
    isError = false;
    try {
      const okMsg = await invoke<string>("export_report", {
        reportJson: JSON.stringify(reportJson),
        outputPath: defaultOutputPath(format),
        format,
      });
      if (!destroyed) message = okMsg;
    } catch (e) {
      if (!destroyed) {
        // 后端已保证中文错误文案，这里只做兜底转字符串。
        message = String(e ?? "生成报告失败，请重试。");
        isError = true;
      }
    } finally {
      if (!destroyed) busy = false;
    }
  }

  function defaultOutputPath(f: Format): string {
    const stamp = new Date().toISOString().slice(0, 10).replace(/-/g, "");
    // 真实保存路径选择对话框将在 P7 接入 tauri-plugin-dialog；
    // 现阶段默认写到用户目录下的固定文件名，保证流程可跑通。
    return `~/论文格式审核报告-${stamp}.${f === "html" ? "html" : "csv"}`;
  }
</script>

<div class="card border border-base-300 shadow-sm">
  <div class="card-body gap-3">
    <h2 class="card-title">导出审核报告</h2>
    <p class="text-sm opacity-70">
      生成一份可以直接发给老师或同学查看的报告文件，不需要安装任何软件。
    </p>

    <div class="join">
      <button
        class="btn btn-outline join-item"
        class:btn-active={format === "html"}
        aria-pressed={format === "html"}
        onclick={() => (format = "html")}
      >网页版（HTML）</button>
      <button
        class="btn btn-outline join-item"
        class:btn-active={format === "csv"}
        aria-pressed={format === "csv"}
        onclick={() => (format = "csv")}
      >表格版（CSV）</button>
    </div>

    <button class="btn btn-primary" disabled={busy || !reportJson} onclick={doExport}>
      {#if busy}
        <span class="loading loading-spinner loading-sm"></span>
        正在生成…
      {:else}
        生成{format === "html" ? "网页版" : "表格版"}报告
      {/if}
    </button>

    {#if message}
      <div class="alert" class:alert-success={!isError} class:alert-error={isError}>
        <span>{message}</span>
      </div>
    {/if}
  </div>
</div>
