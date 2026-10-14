//! P8A 前端 API 层：Tauri invoke / event 的统一封装。
//!
//! 职责边界：
//! - 只做「调用 + 错误归一化」，不含任何业务逻辑与状态（状态在 stores.ts）；
//! - 所有失败路径收敛为 `ApiResult<T>`，`user_message` 保证是可直接展示的中文；
//! - 事件监听统一返回「取消函数」，供组件 `$effect` / store 销毁时清理。

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview, type DragDropEvent } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import type {
  ApiResult,
  AuditOptions,
  AuditReport,
  DoneEvent,
  ErrorEvent,
  FileMeta,
  LogEvent,
  ProgressEvent,
  ReportFormat,
} from "./types";

// =========================
// 错误归一化
// =========================

/** 把任意异常/字符串转成用户可读的中文消息（审查项 9）。 */
export function toUserMessage(e: unknown): string {
  if (typeof e === "string" && e.trim().length > 0) return e;
  if (e instanceof Error && e.message.trim().length > 0) return e.message;
  return "操作没有成功完成，请重试。如果反复出现，请联系指导教师或重新打开本程序。";
}

/** 判断是否为「用户主动取消对话框」——不应视为错误展示。 */
function isDialogCancel(e: unknown): boolean {
  const msg = typeof e === "string" ? e : e instanceof Error ? e.message : "";
  // Tauri dialog 插件在用户点「取消」时 resolve(null)；个别平台会抛含 cancel 的字符串。
  return /cancel/i.test(msg);
}

// =========================
// 命令封装（与后端 commands.rs 一一对应）
// =========================

/** 选择论文所在文件夹；用户取消返回 `{ ok: true, data: null }`。 */
export async function pickInputDir(): Promise<ApiResult<string | null>> {
  try {
    const dir = await openDialog({ directory: true, title: "选择存放论文 Word 文件的文件夹" });
    return { ok: true, data: typeof dir === "string" ? dir : null };
  } catch (e) {
    if (isDialogCancel(e)) return { ok: true, data: null };
    return { ok: false, user_message: "无法打开文件夹选择窗口：" + toUserMessage(e) };
  }
}

/** 选择格式规则文件（.json）；用户取消返回 null。 */
export async function pickRuleFile(): Promise<ApiResult<string | null>> {
  try {
    const file = await openDialog({
      multiple: false,
      title: "选择格式规则文件",
      filters: [{ name: "格式规则", extensions: ["json"] }],
    });
    return { ok: true, data: typeof file === "string" ? file : null };
  } catch (e) {
    if (isDialogCancel(e)) return { ok: true, data: null };
    return { ok: false, user_message: "无法打开文件选择窗口：" + toUserMessage(e) };
  }
}

/** 列出目录下全部 .docx（供 UI 预览，不代表审核范围）。 */
export async function listDocxFiles(dir: string): Promise<ApiResult<FileMeta[]>> {
  try {
    const files = await invoke<FileMeta[]>("list_docx_files", { dir });
    return { ok: true, data: files };
  } catch (e) {
    return { ok: false, user_message: toUserMessage(e) };
  }
}

/**
 * 启动一次批量审核任务。
 * 后端立即返回 job_id，进度/结果通过 `audit://*` 事件推送。
 */
export async function startAudit(options: AuditOptions): Promise<ApiResult<string>> {
  try {
    const jobId = await invoke<string>("start_audit", { options });
    return { ok: true, data: jobId };
  } catch (e) {
    return { ok: false, user_message: toUserMessage(e) };
  }
}

/** 请求取消当前任务（幂等；剩余文件将被标记跳过）。 */
export async function cancelAudit(jobId: string): Promise<ApiResult<null>> {
  try {
    await invoke("cancel_audit", { jobId });
    return { ok: true, data: null };
  } catch (e) {
    return { ok: false, user_message: toUserMessage(e) };
  }
}

/** 导出审核报告（HTML/CSV），返回保存成功提示或写入错误。 */
export async function exportReport(
  report: AuditReport,
  outputPath: string,
  format: ReportFormat,
): Promise<ApiResult<string>> {
  try {
    const msg = await invoke<string>("export_report", {
      reportJson: JSON.stringify(report),
      outputPath,
      format,
    });
    return { ok: true, data: msg };
  } catch (e) {
    return { ok: false, user_message: toUserMessage(e) };
  }
}

/** 用系统默认程序打开文件或文件夹（如打开报告 HTML）。 */
export async function openPathInShell(path: string): Promise<ApiResult<null>> {
  try {
    await openPath(path);
    return { ok: true, data: null };
  } catch (e) {
    return { ok: false, user_message: "无法打开该位置：" + toUserMessage(e) };
  }
}

// =========================
// 事件封装：统一返回取消监听函数（审查项 11）
// =========================

/** 事件名常量，避免拼写漂移。 */
export const AUDIT_EVENTS = {
  progress: "audit://progress",
  log: "audit://log",
  done: "audit://done",
  error: "audit://error",
} as const;

async function bind<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => handler(e.payload));
}

/** 监听 `audit://progress`，返回取消函数。 */
export const onAuditProgress = (h: (p: ProgressEvent) => void) =>
  bind(AUDIT_EVENTS.progress, h);

/** 监听 `audit://log`，返回取消函数。 */
export const onAuditLog = (h: (p: LogEvent) => void) => bind(AUDIT_EVENTS.log, h);

/** 监听 `audit://done`，返回取消函数。 */
export const onAuditDone = (h: (p: DoneEvent) => void) => bind(AUDIT_EVENTS.done, h);

/** 监听 `audit://error`，返回取消函数。 */
export const onAuditError = (h: (p: ErrorEvent) => void) => bind(AUDIT_EVENTS.error, h);

// =========================
// 窗口级文件拖拽（Tauri drag-drop 事件，携带真实路径）
// =========================

/**
 * 监听 Tauri 窗口级的原生文件拖拽事件（需 tauri.conf.json 中 dragDropEnabled=true）。
 * webview 内的 HTML5 DataTransfer 拿不到真实路径，只有这里的 `paths` 是操作系统提供的
 * 绝对路径；enter/drop 时回调完整路径列表，leave 时回调空列表。返回取消函数。
 */
export async function onDragDropPaths(
  handler: (paths: string[], kind: "enter" | "drop" | "leave") => void,
): Promise<ApiResult<UnlistenFn>> {
  try {
    const unlisten = await getCurrentWebview().onDragDropEvent((e) => {
      const p = e.payload;
      if (p.type === "enter" || p.type === "drop") handler(p.paths, p.type);
      else if (p.type === "leave") handler([], "leave");
    });
    return { ok: true, data: unlisten };
  } catch (e) {
    return { ok: false, user_message: "无法启用文件夹拖拽：" + toUserMessage(e) };
  }
}

// 重新导出类型，便于上层（App/stores）标注事件 payload。
export type { DragDropEvent };
