<!--
  RuleSelector.svelte —— 左栏卡片 2：格式规则选择。
  纯展示组件：不调用 Tauri invoke，不解析规则内容。
-->
<script lang="ts">
  import type { AuditRunStatus } from "../lib/types";
  import { ICONS } from "../lib/icons";

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

<section class="card border border-line bg-white" aria-label="格式规则">
  <div class="card-body gap-4">
    <h2 class="font-heading text-[17px] font-semibold text-ink leading-none">格式规则</h2>

    <div class="flex items-center justify-between gap-2">
      <div class="min-w-0">
        {#if ruleFileName}
          <p class="truncate text-sm text-ink-2" title={ruleFileName}>
            当前规则：{ruleFileName}
          </p>
        {:else}
          <p class="text-sm text-ink-2">
            <span class="badge badge-success badge-sm mr-1.5">内置规则</span>
            教育类论文通用格式要求
          </p>
        {/if}
      </div>
      {#if ruleFileName && !disabled}
        <button type="button" class="btn btn-ghost btn-sm shrink-0" onclick={onReset}>
          恢复默认
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
      <span class="icon" style="--icon-size:16px" aria-hidden="true">{@html ICONS.ruler}</span>
      {#if disabled}审核进行中，暂不可更改{:else}选择规则文件{/if}
    </button>

    <p class="text-sm text-subtle">
      学校有单独的格式要求时，选择对应的规则文件；不选则使用内置通用规则。
    </p>
  </div>
</section>

<style>
  .bg-white {
    background-color: var(--bg);
  }
  .text-ink {
    color: var(--ink);
  }
  .text-ink-2 {
    color: var(--ink-2);
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
