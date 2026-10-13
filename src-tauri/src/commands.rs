//! Tauri 命令层。
//!
//! 职责边界：本层只做「参数校验 + 对 format_core 的薄封装 + 事件转发」，
//! 任何审核/报告业务逻辑一律放在 `format_core`，禁止写入本层或前端。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

/// 批量审核进度/结果事件名（与前端 `src/lib/api.ts` 中 `AUDIT_EVENTS` 一一对应）。
const EVENT_PROGRESS: &str = "audit://progress";
const EVENT_LOG: &str = "audit://log";
const EVENT_DONE: &str = "audit://done";
const EVENT_ERROR: &str = "audit://error";

// =========================
// 公共数据结构（与前端 types.ts 对应）
// =========================

/// `start_audit` 的参数（对应前端 `AuditOptions`）。
#[derive(Debug, Clone, Deserialize)]
pub struct AuditOptions {
    pub input_dir: String,
    pub rule_file: String,
    #[serde(default)]
    pub generate_annotated_docx: bool,
}

/// 单个 .docx 生成标注副本的参数（对应前端 `generateAnnotatedDocx`）。
#[derive(Debug, Clone, Deserialize)]
pub struct AnnotateOptions {
    /// 原始论文 .docx 路径。
    pub source_docx: String,
    /// 该文件在本次审核中命中的违规列表（JSON，`format_core::dsl::Violation` 数组）。
    pub violations_json: String,
    /// 标注副本输出路径（.docx）。
    pub output_path: String,
}

/// 目录中的一个 .docx 文件（对应前端 `FileMeta`，供 UI 预览）。
#[derive(Debug, Clone, Serialize)]
pub struct FileMeta {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub modified_unix: u64,
}

/// `audit://progress` 事件负载（每完成一个文件推送一次）。
#[derive(Debug, Clone, Serialize)]
struct ProgressEvent {
    job_id: String,
    completed: usize,
    total: usize,
    current_file: String,
    finished_violations: usize,
}

/// `audit://log` 事件负载（面向用户的中文日志行）。
#[derive(Debug, Clone, Serialize)]
struct LogEvent {
    job_id: String,
    at: u64,
    level: &'static str,
    message: String,
}

/// `audit://done` 事件负载（任务结束/取消后携带最终报告）。
#[derive(Debug, Clone, Serialize)]
struct DoneEvent {
    job_id: String,
    cancelled: bool,
    report: format_core::dsl::AuditReport,
}

/// `audit://error` 事件负载（不可恢复错误，中文可直接展示）。
#[derive(Debug, Clone, Serialize)]
struct ErrorEvent {
    job_id: String,
    user_message: String,
}

// =========================
// 任务注册表与事件转发器
// =========================

/// 单个审核任务句柄：取消令牌（供 `cancel_audit` 使用）。
struct AuditJob {
    cancel: Arc<format_core::audit::CancellationToken>,
}

/// Tauri 托管状态：job_id -> 任务句柄。
#[derive(Default)]
pub struct AuditJobs(Mutex<HashMap<String, AuditJob>>);

/// 把 `format_core::audit::AuditProgress` 回调转成 `audit://*` Tauri 事件的桥。
///
/// 注意：`audit_directory` 要求 `progress: impl AuditProgress + Sync`，因此这里
/// 直接为 `EventForwarder` 本身（而不是 `&EventForwarder`）实现该 trait——
/// 调用方传 `forwarder.clone()`（内部字段全部是 `Arc` / 可克隆的 `AppHandle`，
/// 克隆廉价且共享同一计数状态）。
#[derive(Clone)]
struct EventForwarder {
    app: AppHandle,
    job_id: Arc<str>,
    /// (completed, total)：每完成一个文件自增并推送 `audit://progress`。
    done: Arc<Mutex<(usize, usize)>>,
}

impl EventForwarder {
    fn emit_log(&self, message: String) {
        let payload = LogEvent {
            job_id: self.job_id.to_string(),
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            level: "info",
            message,
        };
        let _ = self.app.emit(EVENT_LOG, &payload);
    }
}

impl format_core::audit::AuditProgress for EventForwarder {
    fn on_file_finished(&self, file: &Path, violation_count: usize) {
        let (completed, total) = {
            let mut guard = match self.done.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            guard.0 += 1;
            (guard.0, guard.1)
        };

        let payload = ProgressEvent {
            job_id: self.job_id.to_string(),
            completed,
            total,
            current_file: file.display().to_string(),
            finished_violations: violation_count,
        };

        let _ = self.app.emit(EVENT_PROGRESS, &payload);
    }

    fn on_log(&self, message: String) {
        self.emit_log(message);
    }
}

/// 自检命令：前端可调用以确认 Rust 核心引擎已正确链接。
#[tauri::command]
pub fn core_version() -> String {
    format_core::version().to_string()
}

/// 报告格式（与前端下拉选项对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportFormat {
    Html,
    Csv,
}

/// P6：把批量审核结果（JSON 序列化的 `AuditReport`）导出为 HTML / CSV 文件。
///
/// - `report_json` 来自 `audit_directory` 返回值的序列化结果；
/// - 渲染逻辑全部在 `format_core::report`，本函数不掺入业务；
/// - 错误返回中文用户提示（`user_message`），技术细节仅进日志。
#[tauri::command]
pub async fn export_report(
    report_json: String,
    output_path: String,
    format: ReportFormat,
) -> Result<String, String> {
    let report: format_core::dsl::AuditReport = serde_json::from_str(&report_json).map_err(|e| {
        format!("审核结果数据有误，无法生成报告：{e}\n（请重新运行一次检查后再导出）")
    })?;

    let path = Path::new(&output_path);

    let result = match format {
        ReportFormat::Html => format_core::report::generate_html_report(&report, path),
        ReportFormat::Csv => format_core::report::generate_csv_report(&report, path),
    };

    match result {
        Ok(()) => Ok(format!("报告已保存到：{output_path}")),
        Err(e) => {
            // 日志留全链路，用户只见中文提示。
            eprintln!("[export_report] {}", e.debug_chain());
            Err(e.user_message())
        }
    }
}

/// 单个 .docx 生成标注副本（供前端「打开标注稿」入口按需调用）。
#[tauri::command]
pub async fn generate_annotated_docx(options: AnnotateOptions) -> Result<String, String> {
    let source = PathBuf::from(&options.source_docx);
    if !source.is_file() {
        return Err("原始论文文件不存在或已被移动，请重新运行检查。".to_string());
    }
    let violations: Vec<format_core::dsl::Violation> =
        serde_json::from_str(&options.violations_json).map_err(|e| {
            format!("违规数据有误，无法生成标注稿：{e}\n（请重新运行一次检查后再试）")
        })?;

    let out = PathBuf::from(&options.output_path);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "无法创建输出文件夹，请检查磁盘权限。")?;
    }

    // 纯 CPU + IO 的 zip/XML 操作放进阻塞线程，避免卡住命令运行时。
    let result = tauri::async_runtime::spawn_blocking(move || {
        format_core::annotated_docx::generate_annotated_docx(&source, &violations, &out)
    })
    .await
    .map_err(|_| "标注稿生成任务意外中断，请重试。".to_string())?;

    match result {
        Ok(stats) => Ok(format!(
            "标注稿已生成：{}（高亮 {} 处）",
            out.display(),
            stats.highlighted_runs
        )),
        Err(e) => {
            eprintln!("[generate_annotated_docx] {}", e.debug_chain());
            Err(e.user_message())
        }
    }
}

/// 列出目录下全部 .docx（供 UI 预览，不代表审核范围）。
#[tauri::command]
pub fn list_docx_files(dir: String) -> Result<Vec<FileMeta>, String> {
    let path = Path::new(&dir);

    if !path.is_dir() {
        return Err("所选位置不是一个文件夹，请重新选择。".to_string());
    }

    let mut files = Vec::new();
    collect_docx(path, &mut files).map_err(|e| e.user_message())?;

    let mut metas: Vec<FileMeta> = files
        .iter()
        .filter_map(|p| {
            let meta = std::fs::metadata(p).ok()?;

            let modified_unix = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            Some(FileMeta {
                path: p.display().to_string(),
                name: p.file_name()?.to_string_lossy().into_owned(),
                size_bytes: meta.len(),
                modified_unix,
            })
        })
        .collect();

    metas.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(metas)
}

/// 递归收集目录下的 .docx 文件（业务规则与 format_core 保持一致：只看扩展名）。
fn collect_docx(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), format_core::error::CoreError> {
    let entries = std::fs::read_dir(dir).map_err(|_| format_core::error::CoreError::InvalidPath {
        path: dir.display().to_string(),
        reason: "无法读取该文件夹内容".into(),
    })?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();

        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if ft.is_dir() {
            collect_docx(&path, out)?;
        } else if ft.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("docx"))
                .unwrap_or(false)
        {
            out.push(path);
        }
    }

    Ok(())
}

/// 启动一次批量审核任务：立即返回 job_id，进度/结果通过 `audit://*` 事件推送。
///
/// 修复点：
/// 1. `audit_directory` 的签名是 `progress: impl AuditProgress + Sync`，而
///    `AuditProgress` 是为 `EventForwarder` 本身实现的；因此这里传
///    `forwarder` 本身，而不是 `&forwarder`。
/// 2. `spawn_blocking(move || ...)` 会拿走闭包中变量所有权，所以进入闭包前
///    先克隆 `app` 和 `job_id`，避免后续 `handle.await` 失败分支再次使用时报 E0382。
/// 3. `try_state` 来自 `tauri::Manager` trait，顶部已导入。
#[tauri::command]
pub async fn start_audit(app: AppHandle, options: AuditOptions) -> Result<String, String> {
    let input_dir = PathBuf::from(&options.input_dir);
    if !input_dir.is_dir() {
        return Err("所选论文文件夹不存在或不可访问，请重新选择。".to_string());
    }

    let rule_path = PathBuf::from(&options.rule_file);
    if !rule_path.is_file() {
        return Err("格式规则文件不存在，请重新选择。".to_string());
    }

    // 配置加载/校验在异步命令线程完成（快），耗时的逐文件审核放进阻塞任务；
    // 失败时同步返回中文错误（前端 startAudit 的 catch 分支展示）。
    let config =
        format_core::rules::load_rule_config(&rule_path).map_err(|e| e.user_message())?;

    let job_id = uuid_job_id();
    let token = Arc::new(format_core::audit::CancellationToken::new());

    // (completed, total)：total 由任务体扫描目录后回填；forwarder 共享同一 Arc 计数。
    let done = Arc::new(Mutex::new((0usize, 0usize)));

    {
        let jobs = app.state::<AuditJobs>();
        let mut map = jobs
            .0
            .lock()
            .map_err(|_| "任务状态损坏，请重启程序后重试。".to_string())?;

        map.insert(
            job_id.clone(),
            AuditJob {
                cancel: token.clone(),
            },
        );
    }

    let forwarder = EventForwarder {
        app: app.clone(),
        job_id: Arc::from(job_id.as_str()),
        done: done.clone(),
    };

    // ★ E0382 修复：`spawn_blocking(move || ...)` 会移动闭包中使用的变量。
    // 后续 `handle.await` 失败分支还需要使用 `app` 和 `job_id`，所以这里先克隆。
    let app_for_task = app.clone();
    let job_id_for_task = job_id.clone();

    let handle = tauri::async_runtime::spawn_blocking(move || {
        audit_job_body(
            app_for_task,
            job_id_for_task,
            input_dir,
            config,
            forwarder,
            token,
            done,
            options.generate_annotated_docx,
        )
    });

    // 任务结果通过 `audit://done` / `audit://error` 事件推送；这里 await 仅用于
    // 捕获 panic 等 JoinError——此时注册表项可能仍在，需要清理，避免泄漏。
    if let Err(e) = handle.await {
        eprintln!("[start_audit] blocking task join error: {e}");

        if let Some(state) = app.try_state::<AuditJobs>() {
            if let Ok(mut map) = state.0.lock() {
                map.remove(&job_id);
            }
        }

        let payload = ErrorEvent {
            job_id: job_id.clone(),
            user_message: "检查任务意外中断，请重试。如果反复出现，请重启程序。".to_string(),
        };

        let _ = app.emit(EVENT_ERROR, &payload);
    }

    Ok(job_id)
}

/// 请求取消当前任务（幂等；剩余文件将被标记跳过）。
#[tauri::command]
pub fn cancel_audit(job_id: String, jobs: State<'_, AuditJobs>) -> Result<(), String> {
    let map = jobs
        .0
        .lock()
        .map_err(|_| "任务状态损坏，请重启程序后重试。".to_string())?;

    match map.get(&job_id) {
        Some(job) => {
            job.cancel.cancel();
            Ok(())
        }
        // 已完成/不存在的任务：取消视为幂等成功，前端无需处理错误。
        None => Ok(()),
    }
}

/// 阻塞线程中执行的真实审核流程：回填 total、调用 `audit_directory_with` 并 emit done/error。
fn audit_job_body(
    app: AppHandle,
    job_id: String,
    input_dir: PathBuf,
    config: format_core::dsl::RuleConfig,
    forwarder: EventForwarder,
    token: Arc<format_core::audit::CancellationToken>,
    done: Arc<Mutex<(usize, usize)>>,
    generate_annotated: bool,
) {
    // 先扫描目录回填总数，让前端 progress 事件里的 total 有意义。
    let mut files = Vec::new();
    if collect_docx(&input_dir, &mut files).is_ok() {
        if let Ok(mut guard) = done.lock() {
            guard.1 = files.len();
        }
    }

    // ★ E0277 修复核心：按值传 `forwarder`（`AuditProgress` 为 `EventForwarder` 本身实现，
    //   满足 `impl AuditProgress + Sync`）；绝不能传 `&forwarder`——引用类型未实现该 trait。
    match format_core::audit::audit_directory_with(&input_dir, &config, forwarder, &token, generate_annotated) {
        Ok(report) => {
            let cancelled = token.is_cancelled();

            let payload = DoneEvent {
                job_id: job_id.clone(),
                cancelled,
                report,
            };

            let _ = app.emit(EVENT_DONE, &payload);
        }
        Err(e) => {
            eprintln!("[start_audit] {}", e.debug_chain());

            let payload = ErrorEvent {
                job_id: job_id.clone(),
                user_message: e.user_message(),
            };

            let _ = app.emit(EVENT_ERROR, &payload);
        }
    }

    // 任务结束，从注册表移除，避免长期运行累积内存。
    //
    // ★ let-chain 修复：不要写：
    //   if let Some(state) = ... && let Ok(mut map) = ...
    // 在 Rust 2021 下会报错；改成嵌套 if let。
    if let Some(state) = app.try_state::<AuditJobs>() {
        if let Ok(mut map) = state.0.lock() {
            map.remove(&job_id);
        }
    }
}

/// 生成一个足够唯一的 job_id（不引入 uuid crate：时间戳 + 计数 + 进程号混合）。
fn uuid_job_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    let seq = SEQ.fetch_add(1, Ordering::Relaxed);

    format!("job-{nanos:x}-{seq:x}-{:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::future::Future;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    static NOOP_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
        noop_waker_clone,
        noop_waker_noop,
        noop_waker_noop,
        noop_waker_noop,
    );

    fn noop_waker_clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &NOOP_WAKER_VTABLE)
    }

    fn noop_waker_noop(_: *const ()) {}

    fn noop_waker() -> Waker {
        unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &NOOP_WAKER_VTABLE)) }
    }

    #[test]
    fn core_version_returns_semver() {
        let v = core_version();

        // 骨架阶段仅要求形如 x.y.z 且非空
        assert!(v.chars().next().is_some_and(|c| c.is_ascii_digit()));
        assert_eq!(v, "0.1.0");
    }

    #[test]
    fn report_format_deserializes_snake_case() {
        let h: ReportFormat = serde_json::from_str("\"html\"").unwrap();
        let c: ReportFormat = serde_json::from_str("\"csv\"").unwrap();

        assert_eq!(h, ReportFormat::Html);
        assert_eq!(c, ReportFormat::Csv);
    }

    /// 非法 JSON 必须得到中文、含引导语的错误，而不是 panic。
    ///
    /// `export_report` 是 async 命令，但全部校验逻辑在任何 await 点之前完成，
    /// 因此手动 poll 一次即为 Ready——测试不引入 tokio 等运行时依赖。
    #[test]
    fn export_report_rejects_bad_json_with_chinese_hint() {
        let out_path = std::env::temp_dir().join("paper_audit_不应被写入.html");
        let _ = std::fs::remove_file(&out_path);

        let fut = export_report(
            "{ 坏数据".to_string(),
            out_path.to_string_lossy().to_string(),
            ReportFormat::Html,
        );

        // 把 future 固定到堆上再轮询到完成（无 unsafe）。
        let mut boxed: std::pin::Pin<Box<dyn Future<Output = Result<String, String>>>> =
            Box::pin(fut);

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        let err = loop {
            match boxed.as_mut().poll(&mut cx) {
                Poll::Ready(Ok(v)) => panic!("期望 Err，却成功返回：{v}"),
                Poll::Ready(Err(e)) => break e,
                Poll::Pending => continue, // 实际不会发生：函数体内无真实异步等待
            }
        };

        assert!(err.contains("审核结果数据有误"), "{err}");
        assert!(
            !out_path.exists(),
            "参数校验失败时不应写出任何报告文件"
        );
    }
}
