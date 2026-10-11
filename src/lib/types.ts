//! P8A 前端类型层。
//!
//! 本文件中的 interface 与 Rust 后端序列化结构一一对应：
//! - `Violation` / `AuditSummary` / `AuditReport` ← `format_core::dsl`（serde snake_case）；
//! - `ApiResult<T>` ← Tauri 命令统一返回 `Result<T, String>`，其中 `String` 为中文 user_message；
//! - 事件负载 `ProgressEvent` / `LogEvent` / `DoneEvent` / `ErrorEvent` ← `audit://*` 事件 emit 的 JSON。
//!
//! 约定：字段名保持 snake_case 以匹配 serde 输出，禁止在前端做二次改名映射。

/** 违规严重级别（对应 Rust `Severity`，serde snake_case）。 */
export type Severity = "error" | "warning" | "info";

/** 单个文件的审核状态（对应 Rust `FileStatus`）。 */
export type FileStatus = "passed" | "needs_revision" | "read_failed" | "skipped";

/** 全局审核阶段状态（store 内部使用，非后端类型）。 */
export type AuditRunStatus = "idle" | "running" | "done" | "cancelled" | "error";

/** 报告导出格式（对应 Rust `ReportFormat`）。 */
export type ReportFormat = "html" | "csv";

/** 违规位置（对应 Rust `Location`，各字段可为 null——文档级问题无具体段落）。 */
export interface Location {
  section_index: number | null;
  block_index: number | null;
  paragraph_index: number | null;
  run_index: number | null;
  /** 页码估算（按字数粗算，仅供定位参考，可能不准）。 */
  page_hint: number | null;
  object_ref: string | null;
}

/** 一条格式问题（对应 Rust `Violation`）。message 已是面向普通用户的中文。 */
export interface Violation {
  rule_id: string;
  severity: Severity;
  file_path: string;
  location: Location;
  message: string;
  expected: string | null;
  actual: string | null;
  excerpt: string | null;
  suggestion: string | null;
  /** 0~1 置信度。 */
  confidence: number;
  requires_manual_review: boolean;
}

/** 单文件审核摘要（对应 Rust `FileAuditSummary`）。 */
export interface AuditSummary {
  file_path: string;
  file_name: string;
  status: FileStatus;
  error_count: number;
  warning_count: number;
  info_count: number;
  violations: Violation[];
  /** 读取失败时的中文原因（损坏/加密/空文件等），不含技术堆栈。 */
  parse_error: string | null;
}

/** 批量审核总报告（对应 Rust `AuditReport`）。 */
export interface AuditReport {
  generated_at: string;
  input_dir: string;
  rule_file: string;
  rule_profile: string | null;
  total_files: number;
  passed_files: number;
  failed_files: number;
  read_failed_files: number;
  total_errors: number;
  total_warnings: number;
  total_infos: number;
  files: AuditSummary[];
}

/** 目录中的一个 .docx 文件（对应命令 `list_docx_files` 返回值）。 */
export interface FileMeta {
  path: string;
  name: string;
  /** 文件大小（字节）。 */
  size_bytes: number;
  /** 最后修改时间（Unix 秒）。 */
  modified_unix: number;
}

/** start_audit 的参数（对应 Rust `AuditOptions`，字段 snake_case 与 serde 一致）。 */
export interface AuditOptions {
  input_dir: string;
  /** null / 缺省 = 使用内置通用规则。 */
  rule_file?: string | null;
  /** 是否一并扫描子文件夹（后端始终递归，此标志用于前端展示与校验）。 */
  include_subdirs?: boolean;
  /** 是否为每个有问题的文件生成标注版 DOCX 副本。 */
  generate_annotated_docx?: boolean;
}

/**
 * Tauri 命令统一返回包装：
 * - `ok: true` 时 `data` 有值；
 * - `ok: false` 时 `user_message` 为可直接展示的中文提示。
 */
export type ApiResult<T> =
  | { ok: true; data: T }
  | { ok: false; user_message: string };

// =========================
// audit://* 事件负载
// =========================

/** `audit://progress`：每完成一个文件推送一次。 */
export interface ProgressEvent {
  job_id: string;
  /** 已完成文件数（含读取失败）。 */
  completed: number;
  /** 计划处理的文件总数。 */
  total: number;
  current_file: string;
  finished_violations: number;
}

/** `audit://log`：面向用户的中文日志行。 */
export interface LogEvent {
  job_id: string;
  /** Unix 毫秒时间戳。 */
  at: number;
  level: "info" | "warn" | "error";
  message: string;
}

/** `audit://done`：任务正常结束或用户取消后携带最终结果。 */
export interface DoneEvent {
  job_id: string;
  /** 用户主动取消时为 true。 */
  cancelled: boolean;
  report: AuditReport;
}

/** `audit://error`：任务因不可恢复错误终止。 */
export interface ErrorEvent {
  job_id: string;
  /** 中文、可直接展示给用户。 */
  user_message: string;
}

/**
 * 一次审核产出的文件路径清单（对应 Rust `ExportedFiles`）。
 * 后端在导出报告 / 生成标注版 DOCX 后随事件或返回值下发，供 ExportBar「打开文件」使用。
 */
export interface ExportedFiles {
  /** 相对 input_dir 的 POSIX 风格路径，如 "论文格式检查报告.html"。 */
  rel_path: string;
  /** 绝对路径（展示与 shell 打开用）。 */
  abs_path: string;
  kind: "html" | "csv" | "annotated_docx";
}
