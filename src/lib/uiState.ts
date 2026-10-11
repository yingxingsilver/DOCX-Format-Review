//! P8B 前端展示状态层（纯逻辑，不依赖 Tauri）。
//!
//! 职责：为 GUI 主布局提供「可本地模拟」的界面状态与动作。
//! - 真实后端接线在 stores.ts（P8A），本阶段（P8B）只做布局与交互，
//!   因此 App 组合根从这里读取状态，便于脱离桌面运行时验证各状态界面；
//! - 后续接入后端时，只需把 App.svelte 的 import 从 uiState 换到 stores，
//!   或在本文件内把 action 转发给 stores 对应函数（调用点已一一对应）。

import { derived, get, writable } from "svelte/store";
import type { AuditRunStatus } from "./types";

// =========================
// 状态（字段命名与任务书/stores 期望一致）
// =========================

export const status = writable<AuditRunStatus>("idle");
export const inputDir = writable<string | null>(null);
export const rulePath = writable<string | null>(null);
/** 规则文件名（由 rulePath 派生，仅展示用）。 */
export const ruleFileName = derived(rulePath, (p) =>
  p ? (p.split(/[\\/]/).filter(Boolean).pop() ?? p) : null,
);
export const includeSubdirs = writable<boolean>(false);
export const generateAnnotatedDocx = writable<boolean>(false);
/** 已发现的 Word 文件数（选择文件夹后更新；审核完成后为实际处理数）。 */
export const fileCount = writable<number | null>(null);
export const errorMessage = writable<string | null>(null);
/** 进度占位数据（running/done 时展示）。 */
export const progress = writable<{ completed: number; total: number }>({
  completed: 0,
  total: 0,
});
/** done 状态的结果摘要占位（后续阶段由后端报告替换）。 */
export const summaryCounts = writable<{
  total: number;
  passed: number;
  needsRevision: number;
  readFailed: number;
} | null>(null);

// =========================
// 本地模拟开关（验收标准 5/6/7：无需后端即可演示各状态）
// =========================

/**
 * 为 true 时，startAudit/cancelAudit 走本地定时器模拟，不触碰 Tauri。
 * 默认开启——当前阶段没有可用的审核后端接线；接入 P8A stores 后置 false。
 */
export const simulateLocally = writable<boolean>(true);

let simulateTimer: ReturnType<typeof setInterval> | null = null;

function stopTimer(): void {
  if (simulateTimer !== null) {
    clearInterval(simulateTimer);
    simulateTimer = null;
  }
}

// =========================
// Actions（与 stores.ts 中的同名动作一一对应）
// =========================

/** 选择论文文件夹。stores.chooseInputDir() 就绪前，此处保留清晰调用点。 */
export async function chooseInputDir(): Promise<void> {
  // TODO(P8C): return stores.chooseInputDir();
  // 本地环境无 Tauri 对话框，使用 prompt 仅供开发期模拟路径展示；
  // 注意：这不是浏览器 alert/confirm 弹窗式交互的一部分，仅兜底输入路径。
  if (get(status) === "running") return;
  const dir = window.prompt("输入要模拟的论文文件夹路径（本地演示用）", "/Users/me/theses");
  if (dir) {
    inputDir.set(dir);
    // 模拟扫描结果：非负伪随机数量，避免写死样例数值作为正式逻辑。
    fileCount.set(10 + Math.floor(Math.random() * 30));
    errorMessage.set(null);
  }
}

/** 选择格式规则文件。调用点：stores.chooseRuleFile()。 */
export async function chooseRuleFile(): Promise<void> {
  // TODO(P8C): return stores.chooseRuleFile();
  if (get(status) === "running") return;
  const p = window.prompt("输入要模拟的规则文件路径（本地演示用）", "school_rules.json");
  if (p) {
    rulePath.set(p);
    errorMessage.set(null);
  }
}

/** 恢复内置通用规则。 */
export function resetRule(): void {
  if (get(status) === "running") return;
  rulePath.set(null);
}

/** 启动批量审核。stores.startAudit() 就绪前按 simulateLocally 决定是否模拟。 */
export async function startAudit(): Promise<void> {
  if (get(status) === "running") return;
  if (!get(inputDir)) return; // 按钮此时应为禁用态，双重保险
  errorMessage.set(null);
  summaryCounts.set(null);
  stopTimer();
  if (!get(simulateLocally)) {
    // TODO(P8C): return stores.startAudit();
    return;
  }
  const total = get(fileCount) ?? 0;
  progress.set({ completed: 0, total });
  status.set("running");
  let done = 0;
  simulateTimer = setInterval(() => {
    done += 1;
    progress.set({ completed: done, total });
    if (done >= total) {
      stopTimer();
      summaryCounts.set({
        total,
        passed: Math.floor(total * 0.4),
        needsRevision: Math.ceil(total * 0.5),
        readFailed: total - Math.floor(total * 0.4) - Math.ceil(total * 0.5),
      });
      status.set("done");
    }
  }, 400);
}

/** 取消审核。调用点：stores.cancelAudit()。 */
export async function cancelAudit(): Promise<void> {
  // TODO(P8C): return stores.cancelAudit();
  if (get(status) !== "running") return;
  stopTimer();
  status.set("cancelled");
}

/** 清空结果，回到 idle（保留已选文件夹与规则）。调用点：stores.resetAll 的部分语义。 */
export function clearResults(): void {
  if (get(status) === "running") return;
  stopTimer();
  status.set("idle");
  errorMessage.set(null);
  summaryCounts.set(null);
  progress.set({ completed: 0, total: 0 });
}

/** 导出 HTML 报告（本阶段占位）。调用点：stores.exportHtml()。 */
export async function exportHtml(): Promise<void> {
  // TODO(P8C): return stores.exportHtml();
}

/** 导出 CSV 清单（本阶段占位）。调用点：stores.exportCsv()。 */
export async function exportCsv(): Promise<void> {
  // TODO(P8C): return stores.exportCsv();
}

/** 演示用：注入一次失败状态（error 界面验收用，正式版由 audit://error 事件写入）。 */
export function demoError(): void {
  stopTimer();
  errorMessage.set("部分文件无法读取，检查未能全部完成。请确认文件夹中的 Word 文件未损坏、未被其他程序占用，然后重新开始检查。");
  status.set("error");
}
