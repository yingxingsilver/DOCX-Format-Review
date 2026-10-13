//! P8A 前端状态层：Svelte store 管理全局审核状态与动作。
//!
//! 职责边界（审查项 3/12）：
//! - UI 组件只读 store + 调 action，不含业务逻辑；
//! - 耗时操作全部经 api.ts 异步下发给 Tauri 后端（rayon 线程池），
//!   本层只维护事件回调写入的轻量状态，绝不阻塞 UI；
//! - 事件监听在模块初始化时注册一次，action 内部按 job_id 过滤过期事件。

import { derived, get, readable, writable, type Readable } from "svelte/store";
import * as api from "./api";
import type {
  AuditReport,
  AuditRunStatus,
  FileMeta,
  Location,
  Violation,
} from "./types";

// =========================
// 基础状态 store（只读导出，写入口集中在本文件 action）
// =========================

export const inputDir = writable<string | null>(null);
export const rulePath = writable<string | null>(null);
/** 恢复内置通用规则（不指定外部 .json 规则文件）。 */
export function resetRule(): void {
  if (get(status) === "running") return;
  rulePath.set(null);
}
/** 审核选项：包含子文件夹 / 生成标注版 DOCX（UI 状态；后端 AuditOptions 就绪前暂不下发）。 */
export const includeSubdirs = writable<boolean>(false);
export const generateAnnotatedDocx = writable<boolean>(false);
export const files = writable<FileMeta[]>([]);
export const jobId = writable<string | null>(null);
export const status = writable<AuditRunStatus>("idle");
export const progress = writable<{ completed: number; total: number }>({ completed: 0, total: 0 });
/** 当前正在处理的文件名（P8C ProgressPanel；由 audit://progress 的 current_file 派生）。 */
export const currentFileName = writable<string | null>(null);
export const logs = writable<string[]>([]);
export const summary = writable<AuditReport | null>(null);
export const selectedFile = writable<string | null>(null);
export const exporting = writable<boolean>(false);
/** 界面提示条（成功/错误各一次消费）。 */
export const notice = writable<{ kind: "success" | "error"; text: string } | null>(null);

// —— P8C 组合根（App.svelte）使用的补充状态，命名与旧 uiState 层对齐 ——
/** 规则文件名（由 rulePath 派生，仅展示用）。 */
export const ruleFileName = derived(rulePath, (p) =>
  p ? (p.split(/[\\/]/).filter(Boolean).pop() ?? p) : null,
);
/** 已发现的 Word 文件数（选择文件夹后更新）。 */
export const fileCount = derived(files, (fs) => fs.length);
/** 最近一次错误提示（error 态界面展示；新一轮动作开始时清空）。 */
export const errorMessage = writable<string | null>(null);
/** done 状态的结果摘要（WorkspacePanel 数据源，由 summary 派生）。 */
export const summaryCounts = derived(summary, (r) =>
  r
    ? {
        total: r.total_files,
        passed: r.passed_files,
        needsRevision: r.failed_files,
        readFailed: r.read_failed_files,
      }
    : null,
);
/** 当前处理阶段文案。真实后端只在任务结束时推送结果，中间过程仅有逐文件进度，
 *  因此这里由进度推导一句人话（如「正在检查第 3 / 20 个文件」），不再本地伪造阶段。 */
export const stageLabel = derived(progress, (p) =>
  p.total > 0 && p.completed < p.total
    ? `正在检查第 ${Math.min(p.completed + 1, p.total)} / ${p.total} 个文件`
    : "",
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
  if (kind === "error") errorMessage.set(text);
}

/** 从 ApiResult 提取错误并展示；成功返回 true。 */
function guard(result: { ok: boolean; user_message?: string }): boolean {
  if (result.ok) return true;
  setNotice("error", result.user_message ?? "操作失败，请重试。");
  return false;
}

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
      const name = p.current_file.split(/[\\/]/).filter(Boolean).pop() ?? p.current_file;
      currentFileName.set(name || null);
    }),
    api.onAuditLog((p) => {
      if (get(jobId) !== p.job_id) return;
      pushLog(p.message);
    }),
    api.onAuditDone((d) => {
      if (get(jobId) !== d.job_id) return;
      summary.set(d.report);
      status.set(d.cancelled ? "cancelled" : "done");
      jobId.set(null);
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
      errorMessage.set(e.user_message);
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
// 默认输出目录（报告与标注版 DOCX 统一放这里）
// =========================

/** 结果输出子目录名：位于所选论文文件夹之下。 */
export const OUTPUT_DIR_NAME = "格式检查输出";

/** 拼接路径（Tauri 各平台均接受 "/" 分隔符，Windows 亦可）。 */
function joinPath(dir: string, name: string): string {
  const sep = dir.endsWith("/") || dir.endsWith("\\") ? "" : "/";
  return `${dir}${sep}${name}`;
}

/** 本次结果的默认输出目录（论文文件夹下的「格式检查输出」）。 */
export function outputDirOf(report: AuditReport): string {
  return joinPath(report.input_dir, OUTPUT_DIR_NAME);
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

/** 选择规则文件。 */
export async function chooseRuleFile(): Promise<void> {
  const res = await api.pickRuleFile();
  if (!guard(res)) return;
  if (res.ok && res.data !== null) rulePath.set(res.data);
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
  await ensureEventSubscriptions();
  summary.set(null);
  logs.set([]);
  progress.set({ completed: 0, total: get(files).length });
  currentFileName.set(null);
  selectedFile.set(null);
  notice.set(null);
  errorMessage.set(null);
  const res = await api.startAudit({ input_dir: dir, rule_file: rule });
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

/** 导出 HTML 报告到默认位置并可选打开。 */
export async function exportHtml(): Promise<void> {
  await doExport("html");
}

/** 导出 CSV 问题清单。 */
export async function exportCsv(): Promise<void> {
  await doExport("csv");
}

async function doExport(format: "html" | "csv"): Promise<void> {
  const report = get(summary);
  if (!report) {
    setNotice("error", "还没有可导出的检查结果，请先运行一次检查。");
    return;
  }
  const ext = format === "html" ? "论文格式检查报告.html" : "论文格式问题清单.csv";
  const outPath = joinPath(outputDirOf(report), ext);
  exporting.set(true);
  try {
    const res = await api.exportReport(report, outPath, format);
    if (guard(res) && res.ok) setNotice("success", res.data);
  } finally {
    exporting.set(false);
  }
}
// =========================
// 结果打开与导出（产物默认写入论文文件夹下的「格式检查输出」目录）
// =========================

/** 用系统默认程序打开最近一次导出的 HTML 报告。 */
export async function openReport(): Promise<void> {
  const report = get(summary);
  if (!report) {
    setNotice("error", "还没有生成过报告，请先运行检查并导出。");
    return;
  }
  const res = await api.openPathInShell(joinPath(outputDirOf(report), "论文格式检查报告.html"));
  guard(res);
}

/** 在系统文件管理器中打开输出文件夹（报告与标注版 DOCX 都在这里）。 */
export async function openAnnotatedFolder(): Promise<void> {
  const report = get(summary);
  if (!report) {
    setNotice("error", "还没有检查结果，请先运行一次检查。");
    return;
  }
  const res = await api.openPathInShell(outputDirOf(report));
  guard(res);
}

/** 选中某文件查看违规详情。 */
export function selectFile(path: string | null): void {
  selectedFile.set(path);
}

/** 清空本次结果回到 idle，但保留已选文件夹与规则（换一批文件重审时用）。 */
export function clearResults(): void {
  if (get(status) === "running") return; // 运行中不允许清空，避免与事件竞态
  status.set("idle");
  errorMessage.set(null);
  notice.set(null);
  progress.set({ completed: 0, total: 0 });
  currentFileName.set(null);
  summary.set(null);
  selectedFile.set(null);
  logs.set([]);
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
