<!--
  RuleSelector.svelte —— 左栏卡片 2：格式规则选择。
  纯展示组件：不调用 Tauri invoke，不解析规则内容。
-->
<script lang="ts">
  import type { AuditRunStatus } from "../lib/types";

  interface Props {
    /** 自定义规则文件名；null 表示使用内置通用规则。 */
    ruleFileName: string | null;
    status: AuditRunStatus;
    /** 规则文件校验失败标记（后续阶段由后端校验结果驱动）。 */
    ruleInvalid?: boolean;
    onChoose: () => void;
    onReset: () => void;
  }

  let { ruleFileName, status, ruleInvalid = false, onChoose, onReset }: Props = $props();

  const disabled = $derived(status === "running");
</script>

<section class="card bg-base-100 border border-base-200 shadow-sm" aria-label="格式规则">
  <div class="card-body gap-3">
    <h2 class="text-lg font-semibold text-base-content">格式规则</h2>

    <div class="flex items-center justify-between gap-2">
      <div class="min-w-0">
        {#if ruleFileName}
          <p class="truncate text-sm text-base-content" title={ruleFileName}>
            当前规则：{ruleFileName}
          </p>
        {:else}
          <p class="text-sm text-base-content">
            <span class="badge badge-success badge-sm mr-1">默认</span>
            使用内置通用规则
          </p>
        {/if}
      </div>
      {#if ruleFileName && !disabled}
        <button type="button" class="btn btn-ghost btn-sm shrink-0" onclick={onReset}>
          恢复默认规则
        </button>
      {/if}
    </div>

    {#if ruleInvalid}
      <div role="alert" class="alert alert-warning py-2 text-sm">
        <span>规则文件格式不正确，请重新选择。</span>
      </div>
    {/if}

    <button
      type="button"
      class="btn btn-outline w-full"
      disabled={disabled}
      onclick={onChoose}
    >
      {#if disabled}审核进行中，暂不可更改{:else}选择格式规则{/if}
    </button>

    <p class="text-sm text-base-content/60">
      可不选择，系统会使用内置教育类论文通用规则。
    </p>
  </div>
</section>
