//! P8D 前端状态层：Svelte store 管理全局审核状态与动作（GUI 完整闭环）。
//!
//! 职责边界（审查项 3/12）：
//! - UI 组件只读 store + 调 action，不含业务逻辑；
//! - 耗时操作全部经 api.ts 异步下发给 Tauri 后端（核心引擎线程池），
//!   本层只维护事件回调写入的轻量状态，绝不阻塞 UI；
//! - 事件监听在模块初始化时注册一次，action 内部按 job_id 过滤过期事件。
//!
//! P8D 新增：
//! - 界面状态机 idle / running / done / error / cancelled（status 字段直接承载）；
//! - 导出结果登记（lastExports）：HTML / CSV / 标注版 DOCX 路径 + “打开文件”支持；
//! - 审核选项（含子文件夹、生成标注副本）随 start_audit 下发后端；
//! - 窗口关闭兜底取消（beforeunload → cancelAudit），配合 Rust 端窗口事件不产生僵尸任务。

import { derived, get, readable, writable, type Readable } from "svelte/store";
import * as api from "./api";
import type {
  AnnotatedOutcome,
  AuditReport,
  AuditRunStatus,
  FileMeta,
  Location,
  ReportFormat,
  Violation,
} from "./types";

// =========================
// 基础状态 store（只读导出，写入口集中在本文件 action）
// =========================

export const inputDir = writable<string | null>(null);
export const rulePath = writable<string | null>(null);
export const files = writable<FileMeta[]>([]);
export const jobId = writable<string | null>(null);
/** 界面状态机：idle | running | done | error | cancelled。 */
export const status = writable<AuditRunStatus>("idle");
export const progress = writable<{ completed: number; total: number }>({ completed: 0, total: 0 });
/** 当前正在处理的文件名（P8C ProgressPanel；由 audit://progress 的 current_file 派生）。 */
export const currentFileName = writable<string | null>(null);
/** 运行日志（字符串行；LogPanel「运行详情（可选）」数据源）。 */
export const logs = writable<string[]>([]);
/** 最近一次审核报告（done/cancelled 后写入；ResultTable / ExportBar 数据源）。 */
export const summary = writable<AuditReport | null>(null);
export const selectedFile = writable<string | null>(null);
/** 导出进行中标志（ExportBar loading）。 */
export const exporting = writable<boolean>(false);
/** 界面提示条（成功/错误各一次消费）。 */
export const notice = writable<{ kind: "success" | "error"; text: string } | null>(null);

// =========================
// P8D：审核选项 / 规则校验 / 导出登记
// =========================

/** 是否包含子文件夹（透传给后端；核心层当前始终递归，保留界面意图）。 */
export const includeSubdirs = writable<boolean>(true);
/** 是否为有问题的文件生成黄色高亮标注副本。 */
export const generateAnnotatedDocx = writable<boolean>(false);
/** 规则文件校验状态（选择后立即校验，开始审核前给用户反馈）。 */
export const ruleValidation = writable<{
  state: "unchecked" | "checking" | "ok" | "invalid";
  message: string;
}>({ state: "unchecked", message: "" });
/** 最近一次导出的产物路径（键：html / csv）。 */
export const lastExports = writable<Record<string, string>>({});

/** 派生：标注副本清单（从报告的附加信息中解析；未生成时为空数组）。 */
export const annotatedOutcomes: Readable<AnnotatedOutcome[]> = derived(summary, (report) => {
  const raw = report?.rule_profile;
  if (!raw || !raw.startsWith("[")) return [];
  try {
    const parsed = JSON.parse(raw) as AnnotatedOutcome[];
    return Array.isArray(parsed) ? parsed.filter((o) => o && typeof o.file_path === "string") : [];
  } catch {
    return [];
  }
});

/** 派生：已成功生成的标注副本列表（ExportBar「打开标注版 DOCX」按钮数据源）。 */
export const annotatedFiles: Readable<AnnotatedOutcome[]> = derived(annotatedOutcomes, (list) =>
  list.filter((o) => o.ok),
);

/** 派生只读 store：当前选中文件的违规列表（O(1) 查找表构建一次/报告）。 */
export const violationsByFile: Readable<Map<string, Violation[]>> = readable(
  new Map<string, Violation[]>(),
  (set) => {
    const unsub = summary.subscribe((report) => {
      const map = new Map<string, Violation[]>();
      if (report) {
        for (const f of report.files) map.set(f.file_path, f.violations);
      }
      set(map);
    });
    return unsub;
  },
);

// =========================
// 内部工具
// =========================

const MAX_LOG_LINES = 500;

function pushLog(line: string): void {
  logs.update((ls) => {
    // 上限保护：防止长时间批量任务把内存吃满（审查项 13）。
    const next = ls.length >= MAX_LOG_LINES ? ls.slice(ls.length - MAX_LOG_LINES + 1) : ls.slice();
    next.push(line);
    return next;
  });
}

function setNotice(kind: "success" | "error", text: string): void {
  notice.set({ kind, text });
}

/** 从 ApiResult 提取错误并展示；成功返回 true。 */
function guard(result: { ok: boolean; user_message?: string }): boolean {
  if (result.ok) return true;
  setNotice("error", result.user_message ?? "操作失败，请重试。");
  return false;
}

/** 取路径的文件名（兼容 Windows / POSIX 分隔符）。 */
export function pathBaseName(p: string): string {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
}

/** 拼接目录与文件名的安全分隔（Windows 目录用反斜杠）。 */
function joinPath(dir: string, name: string): string {
  if (dir.endsWith("/") || dir.endsWith("\\")) return `${dir}${name}`;
  const sep = dir.includes("\\") ? "\\" : "/";
  return `${dir}${sep}${name}`;
}

export const HTML_REPORT_NAME = "论文格式检查报告.html";
export const CSV_REPORT_NAME = "论文格式问题清单.csv";

// =========================
// 事件接线（模块加载时注册一次；unlisten 保留供测试/热更新场景）
// =========================

let unlistenAll: (() => void) | null = null;
// 同步置位 + Promise 双保护：防止并发调用重复注册监听。
let subscribing: Promise<void> | null = null;

async function ensureEventSubscriptions(): Promise<void> {
  if (unlistenAll || subscribing) return;
  // 注意：task 体内不得 await 除 listen 之外的东西；subscribing 同步置位，
  // 保证并发调用共享同一次注册。
  const task = Promise.all([
    api.onAuditProgress((p) => {
      // 丢弃过期任务的进度事件（上一轮 done 后、新一轮 start 前的迟到包）。
      if (get(jobId) !== p.job_id) return;
      progress.set({ completed: p.completed, total: p.total });
      // P8C ProgressPanel 需要展示当前文件名；后端 current_file 为完整路径，取文件名。
      currentFileName.set(pathBaseName(p.current_file) || null);
    }),
    api.onAuditLog((e) => {
      if (get(jobId) !== e.job_id) return;
      pushLog(e.message);
    }),
    api.onAuditDone((d) => {
      if (get(jobId) !== d.job_id) return;
      summary.set(d.report);
      status.set(d.cancelled ? "cancelled" : "done");
      jobId.set(null);
      currentFileName.set(null);
      setNotice(
        "success",
        d.cancelled
          ? "检查已停止，下面显示的是已完成部分的结果。"
          : `检查完成：共 ${d.report.total_files} 个文件，其中 ${d.report.failed_files} 个需要修改。`,
      );
    }),
    api.onAuditError((e) => {
      if (get(jobId) !== e.job_id) return;
      status.set("error");
      jobId.set(null);
      currentFileName.set(null);
      setNotice("error", e.user_message);
    }),
  ]).then((unsubs) => {
    subscribing = null;
    // dispose 在 await 期间发生过：立即释放本次监听，避免泄漏。
    if (unlistenAll) {
      for (const u of unsubs) if (typeof u === "function") u();
      return;
    }
    unlistenAll = () => {
      for (const u of unsubs) if (typeof u === "function") u();
      unlistenAll = null;
    };
  });
  subscribing = task;
  return task;
}

/** 显式释放全部事件监听（应用卸载/单测 teardown 用）。 */
export function disposeAuditEvents(): void {
  subscribing = null; // 使在途的 ensureEventSubscriptions 结果作废并允许重注册
  unlistenAll?.();
}

// =========================
// Actions
// =========================

/** 选择论文文件夹，成功后自动刷新文件列表。 */
export async function chooseInputDir(): Promise<void> {
  const res = await api.pickInputDir();
  if (!guard(res)) return;
  if (res.ok && res.data === null) return; // 用户点了取消，不算错误
  if (res.ok && res.data !== null) {
    inputDir.set(res.data);
    await refreshFileList();
  }
}

/** 选择规则文件；选定后立即校验，无效规则当场中文提示。 */
export async function chooseRuleFile(): Promise<void> {
  const res = await api.pickRuleFile();
  if (!guard(res)) return;
  if (res.ok && res.data !== null) {
    rulePath.set(res.data);
    await validateRule();
  }
}

/** 校验当前规则文件（后端加载 + 结构检查），结果写入 ruleValidation。 */
export async function validateRule(): Promise<void> {
  const rule = get(rulePath);
  if (!rule) {
    ruleValidation.set({ state: "unchecked", message: "" });
    return;
  }
  ruleValidation.set({ state: "checking", message: "正在检查规则文件…" });
  const res = await api.validateRuleFile(rule);
  if (res.ok) {
    ruleValidation.set({ state: "ok", message: `规则「${res.data}」已就绪。` });
  } else {
    ruleValidation.set({ state: "invalid", message: res.user_message });
    setNotice("error", res.user_message);
  }
}

/** 恢复内置默认规则（清空自定义选择；不选则使用通用高校规则）。 */
export function resetRule(): void {
  if (get(status) === "running") return;
  rulePath.set(null);
  ruleValidation.set({ state: "unchecked", message: "" });
}

/** 重新扫描所选文件夹中的 .docx 列表。 */
export async function refreshFileList(): Promise<void> {
  const dir = get(inputDir);
  if (!dir) {
    setNotice("error", "请先选择存放论文 Word 文件的文件夹。");
    return;
  }
  const res = await api.listDocxFiles(dir);
  if (!guard(res)) return;
  if (res.ok) {
    files.set(res.data);
    if (res.data.length === 0) {
      setNotice("error", "这个文件夹里没有发现 Word 论文文件（.docx），请确认选对了位置。");
    }
  }
}

/** 启动批量审核。running 期间禁止重复启动。 */
export async function startAudit(): Promise<void> {
  const s = get(status);
  if (s === "running") return;
  const dir = get(inputDir);
  const rule = get(rulePath);
  if (!dir) {
    setNotice("error", "请先选择存放论文 Word 文件的文件夹。");
    return;
  }
  if (!rule) {
    setNotice("error", "请先选择格式规则文件（.json）。");
    return;
  }
  if (get(ruleValidation).state === "invalid") {
    setNotice("error", "当前规则文件有问题，请按提示修正或换一份规则后再开始。");
    return;
  }
  await ensureEventSubscriptions();
  summary.set(null);
  logs.set([]);
  progress.set({ completed: 0, total: get(files).length });
  currentFileName.set(null);
  selectedFile.set(null);
  lastExports.set({});
  notice.set(null);
  const res = await api.startAudit({
    input_dir: dir,
    rule_file: rule,
    include_subdirs: get(includeSubdirs),
    generate_annotated_docx: get(generateAnnotatedDocx),
  });
  if (!guard(res)) return;
  if (res.ok) {
    jobId.set(res.data);
    status.set("running");
  }
}

/** 取消当前审核（剩余文件将被跳过，已完成部分照常出结果）。 */
export async function cancelAudit(): Promise<void> {
  const id = get(jobId);
  if (!id || get(status) !== "running") return;
  const res = await api.cancelAudit(id);
  guard(res); // 失败仅提示，不改变 running 状态——由 done/error 事件收尾
  currentFileName.set(null);
}

// =========================
// P8D：导出闭环（HTML / CSV / 打开位置）
// =========================

async function runExport(format: ReportFormat, fileName: string): Promise<void> {
  const report = get(summary);
  if (!report) {
    setNotice("error", "还没有可导出的检查结果，请先运行一次检查。");
    return;
  }
  const outPath = joinPath(report.input_dir, fileName);
  exporting.set(true);
  try {
    const res = await api.exportReport(report, outPath, format);
    if (guard(res) && res.ok) {
      lastExports.update((m) => ({ ...m, [format]: outPath }));
      setNotice("success", res.data);
    }
  } finally {
    exporting.set(false);
  }
}

/** 导出 HTML 报告到论文文件夹。 */
export async function exportHtml(): Promise<void> {
  await runExport("html", HTML_REPORT_NAME);
}

/** 导出 CSV 问题清单到论文文件夹。 */
export async function exportCsv(): Promise<void> {
  await runExport("csv", CSV_REPORT_NAME);
}

/** 打开最近一次导出的 HTML 报告（尚未导出则先导出再打开）。 */
export async function openReport(): Promise<void> {
  let path = get(lastExports)["html"];
  if (!path) {
    await exportHtml();
    path = get(lastExports)["html"];
    if (!path) return; // 导出失败已有错误提示
  }
  const res = await api.openPathInShell(path);
  guard(res);
}

/** 打开报告所在文件夹（论文文件夹）。 */
export async function openReportFolder(): Promise<void> {
  const report = get(summary);
  if (!report) {
    setNotice("error", "还没有检查结果，请先运行一次检查。");
    return;
  }
  const res = await api.openPathInShell(report.input_dir);
  guard(res);
}

/** 打开某个文件（原文件或标注副本）；权限不足时中文提示而不是崩溃。 */
export async function openFileAt(path: string): Promise<void> {
  const res = await api.openPathInShell(path);
  guard(res);
}

/** 打开第一个标注版 DOCX（若生成了多个，其余可在结果表中逐文件打开）。 */
export async function openAnnotatedDocx(): Promise<void> {
  const list = get(annotatedFiles);
  if (list.length === 0) {
    setNotice("error", "没有生成标注版文件。开始检查前可以勾选“生成标注副本”。");
    return;
  }
  await openFileAt(list[0].file_path);
  if (list.length > 1) {
    pushLog(`另有 ${list.length - 1} 个标注副本，可在结果表中逐个打开。`);
  }
}

/** 选中某文件查看违规详情。 */
export function selectFile(path: string | null): void {
  selectedFile.set(path);
}

/** 清空结果回到配置界面（done/cancelled/error → idle）。 */
export function clearResults(): void {
  if (get(status) === "running") return;
  summary.set(null);
  selectedFile.set(null);
  lastExports.set({});
  notice.set(null);
  status.set("idle");
}

/** 一键回到初始状态（换文件夹重审时用）。 */
export function resetAll(): void {
  if (get(status) === "running") return; // 运行中不允许重置，避免竞态
  inputDir.set(null);
  rulePath.set(null);
  files.set([]);
  jobId.set(null);
  status.set("idle");
  progress.set({ completed: 0, total: 0 });
  currentFileName.set(null);
  logs.set([]);
  summary.set(null);
  selectedFile.set(null);
  notice.set(null);
  lastExports.set({});
  ruleValidation.set({ state: "unchecked", message: "" });
}

// =========================
// 窗口关闭兜底（要求 3：不造成僵尸任务）
// =========================

/**
 * webview 卸载钩子：任务仍在运行时尽力发起取消请求。
 * 说明：退出确认与最终兜底由 Rust 端窗口关闭事件处理（lib.rs）；
 * 这里只是提前一步礼貌通知后端，即使送达失败也不会残留僵尸任务。
 */
let unloadBound = false;
export function bindWindowCloseGuard(): void {
  if (unloadBound || typeof window === "undefined") return;
  unloadBound = true;

  // 「审核仍在进行，确定退出吗？」提示条（要求 3 的可见部分）。
  // 原生对话框需要额外的窗口关闭权限，为避免权限面扩大，这里用界面内提示 +
  // 立即取消任务的组合达成同等效果：用户能看到发生了什么，且绝不产生僵尸任务。
  const showExitNotice = () => {
    setNotice(
      "error",
      "审核仍在进行中，已随窗口关闭停止本次检查；重新打开程序后可再次开始审核。",
    );
  };

  window.addEventListener("beforeunload", () => {
    if (get(status) === "running") {
      showExitNotice();
      // fire-and-forget：webview 销毁时不保证送达，后端窗口事件是最终兜底。
      void api.cancelAudit(get(jobId) ?? "");
    }
  });
  window.addEventListener("pagehide", () => {
    if (get(status) === "running") {
      void api.cancelAudit(get(jobId) ?? "");
    }
  });
}

// =========================
// 纯函数（供 UI 展示格式化，单独导出便于单测）
// =========================

/** 严重级别 → 中文标签。 */
export function severityLabel(v: Violation): string {
  return v.severity === "error" ? "错误" : v.severity === "warning" ? "警告" : "提示";
}

/** 位置 → 人话（“大约第 3 页 · 第 12 段”；文档级问题返回“整篇文档”）。 */
export function locationLabel(loc: Location): string {
  const parts: string[] = [];
  if (loc.page_hint != null) parts.push(`大约第 ${loc.page_hint} 页`);
  if (loc.paragraph_index != null) parts.push(`第 ${loc.paragraph_index + 1} 段`);
  return parts.length > 0 ? parts.join(" · ") : "整篇文档";
}
