//! P8B 前端展示状态层（纯逻辑，不依赖 Tauri）。
//!
//! 职责：为 GUI 主布局提供「可本地模拟」的界面状态与动作。
//! - 真实后端接线在 stores.ts（P8A），本阶段（P8B）只做布局与交互，
//!   因此 App 组合根从这里读取状态，便于脱离桌面运行时验证各状态界面；
//! - 后续接入后端时，只需把 App.svelte 的 import 从 uiState 换到 stores，
//!   或在本文件内把 action 转发给 stores 对应函数（调用点已一一对应）。

import { derived, get, writable } from "svelte/store";
import type { AuditReport, AuditRunStatus, Violation } from "./types";

/** 运行日志行（P8C LogPanel 使用；真实后端由 audit://log 事件写入）。 */
export interface UiLogLine {
  /** Unix 毫秒时间戳。 */
  at: number;
  level: "info" | "warn" | "error";
  message: string;
}

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
// P8C：进度与结果展示所需状态（命名与 stores.ts 对齐）
// =========================

/** 当前正在处理的文件名（真实后端来自 audit://progress 的 current_file）。 */
export const currentFileName = writable<string | null>(null);
/** 当前处理阶段（人话描述，供 ProgressPanel 展示）。 */
export const stageLabel = writable<string>("");
/** 完整审核报告（done/cancelled 时写入；ResultTable 数据源）。 */
export const report = writable<AuditReport | null>(null);
/** 当前选中查看详情的文件路径。 */
export const selectedFile = writable<string | null>(null);
/** 运行日志（面向高级用户；默认 LogPanel 折叠不可见）。 */
export const logs = writable<UiLogLine[]>([]);

/** 派生：按文件路径索引的违规列表（与 stores.violationsByFile 同名同义）。 */
export const violationsByFile = derived(report, (r) => {
  const map = new Map<string, Violation[]>();
  if (r) for (const f of r.files) map.set(f.file_path, f.violations);
  return map;
});

const MAX_LOG_LINES = 500;

/** 追加一条运行日志（上限保护，防止长任务吃满内存）。 */
export function pushLog(level: UiLogLine["level"], message: string): void {
  logs.update((ls) => {
    const next =
      ls.length >= MAX_LOG_LINES ? ls.slice(ls.length - MAX_LOG_LINES + 1) : ls.slice();
    next.push({ at: Date.now(), level, message });
    return next;
  });
}

/** 选中某文件查看违规详情（stores.selectFile 同名调用点）。 */
export function selectFile(path: string | null): void {
  selectedFile.set(path);
}

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
  report.set(null);
  selectedFile.set(null);
  logs.set([]);
  stopTimer();
  if (!get(simulateLocally)) {
    // TODO(P8C): return stores.startAudit();
    return;
  }
  const total = get(fileCount) ?? 0;
  progress.set({ completed: 0, total });
  stageLabel.set("正在解析样式");
  pushLog("info", "开始检查：共 " + total + " 个文件");
  status.set("running");
  const names = demoFileNames(total);
  let done = 0;
  simulateTimer = setInterval(() => {
    done += 1;
    progress.set({ completed: done, total });
    currentFileName.set(names[done - 1] ?? null);
    stageLabel.set(done <= Math.ceil(total * 0.3) ? "正在解析样式" : done < total ? "正在检查字号" : "正在生成报告");
    if (done % 5 === 0 || done === total) {
      pushLog("info", `已处理 ${done}/${total}：${names[done - 1] ?? ""}`);
    }
    if (done >= total) {
      stopTimer();
      const rep = buildDemoReport(get(inputDir) ?? "", names);
      report.set(rep);
      summaryCounts.set({
        total: rep.total_files,
        passed: rep.passed_files,
        needsRevision: rep.failed_files,
        readFailed: rep.read_failed_files,
      });
      stageLabel.set("");
      pushLog("info", `检查完成：${rep.failed_files} 个文件需要修改`);
      status.set("done");
    }
  }, 400);
}

/** 取消审核。调用点：stores.cancelAudit()。 */
export async function cancelAudit(): Promise<void> {
  // TODO(P8C): return stores.cancelAudit();
  if (get(status) !== "running") return;
  stopTimer();
  stageLabel.set("");
  currentFileName.set(null);
  pushLog("warn", "用户取消了本次检查");
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
  report.set(null);
  selectedFile.set(null);
  currentFileName.set(null);
  stageLabel.set("");
  logs.set([]);
}

// =========================
// P8C 本地演示数据装配（仅 simulateLocally=true 时参与；真实后端接入后删除）
// =========================

/** 由所选目录派生一组可区分的演示文件名（数量跟随 fileCount，非写死样例逻辑）。 */
function demoFileNames(total: number): string[] {
  const dir = get(inputDir) ?? "/papers";
  return Array.from({ length: total }, (_, i) => `${dir}/论文_${String(i + 1).padStart(2, "0")}.docx`);
}

/**
 * 把演示进度数据组装成与后端 AuditReport 完全同构的对象，
 * 使 ResultTable / ViolationDetail 在本地即可走通真实数据结构。
 * 违规条目为占位模板（阶段验证用），字段结构与 serde 输出一致。
 */
function buildDemoReport(dir: string, paths: string[]): AuditReport {
  const files = paths.map((p, i) => {
    const name = p.split(/[\\/]/).pop() ?? p;
    // 每 7 个文件安排一个读取失败，其余交替 通过 / 需修改。
    if (i % 7 === 6) {
      return {
        file_path: p,
        file_name: name,
        status: "read_failed" as const,
        error_count: 0,
        warning_count: 0,
        info_count: 0,
        violations: [],
        parse_error: "文件无法打开，可能已损坏、被加密或正被其他程序占用。请另存一份后再试。",
      };
    }
    if (i % 3 === 0) {
      return {
        file_path: p,
        file_name: name,
        status: "passed" as const,
        error_count: 0,
        warning_count: 0,
        info_count: 0,
        violations: [],
        parse_error: null,
      };
    }
    const mk = (
      rule_id: string,
      severity: "error" | "warning" | "info",
      page: number,
      para: number,
      message: string,
      expected: string,
      actual: string,
      excerpt: string,
      suggestion: string,
    ): Violation => ({
      rule_id,
      severity,
      file_path: p,
      location: {
        section_index: null,
        block_index: null,
        paragraph_index: para - 1,
        run_index: null,
        page_hint: page,
        object_ref: null,
      },
      message,
      expected,
      actual,
      excerpt,
      suggestion,
      confidence: 0.9,
      requires_manual_review: false,
    });
    const violations: Violation[] = [
      mk(
        "font.size.body",
        "error",
        1 + (i % 3),
        3,
        "正文第一段字号偏小，阅读困难。",
        "小四（12 磅）",
        "9 磅",
        "本文以在线学习行为数据为基础……",
        "选中该段文字，将字号调整为“小四”。",
      ),
      mk(
        "spacing.line.body",
        "warning",
        2 + (i % 2),
        8,
        "部分段落行距与学校要求不一致。",
        "固定值 20 磅",
        "单倍行距",
        "研究结果表明，干预措施对成绩提升具有显著作用。",
        "全选正文段落后，在“段落-行距”中改为固定值 20 磅。",
      ),
      mk(
        "heading.style.level1",
        "info",
        1,
        1,
        "一级标题未使用统一的标题样式。",
        "黑体三号居中",
        "宋体四号左对齐",
        "第一章 绪论",
        "选中标题，套用“标题 1”样式并设为黑体三号居中。",
      ),
    ];
    return {
      file_path: p,
      file_name: name,
      status: "needs_revision" as const,
      error_count: violations.filter((v) => v.severity === "error").length,
      warning_count: violations.filter((v) => v.severity === "warning").length,
      info_count: violations.filter((v) => v.severity === "info").length,
      violations,
      parse_error: null,
    };
  });
  const passed = files.filter((f) => f.status === "passed").length;
  const failed = files.filter((f) => f.status === "needs_revision").length;
  const readFailed = files.filter((f) => f.status === "read_failed").length;
  const allV = files.flatMap((f) => f.violations);
  return {
    generated_at: new Date().toISOString(),
    input_dir: dir,
    rule_file: get(rulePath) ?? "内置通用规则",
    rule_profile: null,
    total_files: files.length,
    passed_files: passed,
    failed_files: failed,
    read_failed_files: readFailed,
    total_errors: allV.filter((v) => v.severity === "error").length,
    total_warnings: allV.filter((v) => v.severity === "warning").length,
    total_infos: allV.filter((v) => v.severity === "info").length,
    files,
  };
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
