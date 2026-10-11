//! Tauri 命令层（P8D GUI 整合）。
//!
//! 职责边界：本层只做「参数校验 + 对 format_core 的薄封装 + 事件转发」，
//! 任何审核/报告业务逻辑一律放在 `format_core`，禁止写入本层或前端。
//!
//! P8D 约定：
//! - `start_audit` 立即返回 job_id；耗时审核在 `spawn_blocking` 线程池执行，
//!   进度/日志/结果通过 `audit://*` 事件推送（负载字段与前端 types.ts 一一对应）；
//! - 取消令牌集中登记在 `AuditJobs`，窗口关闭时由 lib.rs 统一触发取消，
//!   避免僵尸任务；
//! - 所有错误返回中文 user_message，技术细节只进 stderr 日志。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use format_core::audit::{AuditProgress, CancellationToken};
use format_core::dsl::AuditReport;

/// 自检命令：前端可调用以确认 Rust 核心引擎已正确链接。
#[tauri::command]
pub fn core_version() -> String {
    format_core::version().to_string()
}

// =========================
// 批量审核任务管理（P8D）
// =========================

/// 全局任务登记表：job_id → 取消令牌。
#[derive(Default)]
pub struct AuditJobs(Mutex<HashMap<String, Arc<CancellationToken>>>);

impl AuditJobs {
    /// 取消全部在途任务（窗口关闭时调用，保证不产生僵尸任务）。返回取消数量。
    pub fn cancel_all(&self) -> usize {
        let map = self.0.lock().unwrap();
        for token in map.values() {
            token.cancel();
        }
        map.len()
    }

    fn register(&self, id: String, token: Arc<CancellationToken>) {
        self.0.lock().unwrap().insert(id, token);
    }

    fn finish(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }

    fn get(&self, id: &str) -> Option<Arc<CancellationToken>> {
        self.0.lock().unwrap().get(id).cloned()
    }
}

/// 审核启动参数（与前端 types.ts `AuditOptions` 对应，serde snake_case）。
#[derive(Debug, Clone, Deserialize)]
pub struct AuditOptions {
    pub input_dir: String,
    pub rule_file: String,
    /// 预留：是否递归子文件夹（当前核心层始终递归，见 collect_docx_files）。
    #[serde(default)]
    pub include_subdirs: bool,
    /// 是否为每个有问题的文件生成黄色高亮标注副本。
    #[serde(default)]
    pub generate_annotated_docx: bool,
}

/// 目录中的一个 .docx 文件（与前端 types.ts `FileMeta` 对应）。
#[derive(Debug, Clone, Serialize)]
pub struct FileMeta {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub modified_unix: u64,
}

/// 单个文件的标注副本生成结果（与前端 types.ts `AnnotatedOutcome` 对应）。
#[derive(Debug, Clone, Serialize)]
pub struct AnnotatedOutcome {
    pub file_path: String,
    pub ok: bool,
    /// 成功时为副本路径；失败时为中文原因。
    pub message: String,
}

fn next_job_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!("job-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed))
}

/// 列出目录下的 .docx 文件（供 UI 预览数量，不代表审核范围）。
#[tauri::command]
pub async fn list_docx_files(dir: String) -> Result<Vec<FileMeta>, String> {
    if dir.trim().is_empty() {
        return Err("请先选择存放论文 Word 文件的文件夹。".into());
    }
    let path = PathBuf::from(&dir);
    let files = format_core::audit::collect_docx_files(&path)
        .map_err(|e| e.user_message())?
        .into_iter()
        .filter_map(|p| {
            let meta = std::fs::metadata(&p).ok()?;
            let modified_unix = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Some(FileMeta {
                path: p.to_string_lossy().to_string(),
                name: p.file_name()?.to_string_lossy().to_string(),
                size_bytes: meta.len(),
                modified_unix,
            })
        })
        .collect();
    Ok(files)
}

/// 校验规则文件能否正常加载（选择规则后立即反馈，避免开始审核才报错）。
#[tauri::command]
pub async fn validate_rule_file(path: String) -> Result<String, String> {
    let config = format_core::rules::load_rule_config(&path)
        .map_err(|e| e.user_message())?;
    format_core::rules::validate_rule_config(&config).map_err(|e| e.user_message())?;
    Ok(config.name.clone())
}

/// 事件转发的进度回调：把核心层回调翻译成 `audit://*` 事件发给前端。
struct EventForwarder {
    app: AppHandle,
    job_id: String,
    total: usize,
    completed: Arc<Mutex<usize>>,
}

impl AuditProgress for EventForwarder {
    fn on_file_started(&self, file: &Path) {
        let _ = self.app.emit(
            "audit://progress",
            serde_json::json!({
                "job_id": self.job_id,
                "completed": *self.completed.lock().unwrap(),
                "total": self.total,
                "current_file": file.to_string_lossy(),
                "finished_violations": 0,
            }),
        );
    }

    fn on_file_finished(&self, file: &Path, violation_count: usize) {
        let done = {
            let mut c = self.completed.lock().unwrap();
            *c += 1;
            *c
        };
        let _ = self.app.emit(
            "audit://progress",
            serde_json::json!({
                "job_id": self.job_id,
                "completed": done,
                "total": self.total,
                "current_file": file.to_string_lossy(),
                "finished_violations": violation_count,
            }),
        );
    }

    fn on_log(&self, message: String) {
        let _ = self.app.emit(
            "audit://log",
            serde_json::json!({
                "job_id": self.job_id,
                "at": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                "level": "info",
                "message": message,
            }),
        );
    }
}

/// 为存在格式问题的文件生成黄色高亮标注副本（原文件不修改）。
fn annotate_files(report: &AuditReport) -> Vec<AnnotatedOutcome> {
    let mut out = Vec::new();
    for f in &report.files {
        if f.violations.is_empty() {
            continue;
        }
        let src = f.file_path.clone();
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "文档".to_string());
        let dst = src.with_file_name(format!("{stem}_标注.docx"));
        let (ok, message) =
            match format_core::annotated_docx::generate_annotated_docx(&src, &f.violations, &dst)
            {
                Ok(stats) => (
                    true,
                    format!(
                        "{}：标注副本已生成（加亮 {} 处文本）。",
                        f.file_name, stats.highlighted_runs
                    ),
                ),
                Err(e) => (false, format!("{}：{}", f.file_name, e.user_message())),
            };
        out.push(AnnotatedOutcome {
            file_path: if ok {
                dst.to_string_lossy().to_string()
            } else {
                src.to_string_lossy().to_string()
            },
            ok,
            message,
        });
    }
    out
}

/// 启动一次批量审核：立即返回 job_id，结果经 `audit://done` / `audit://error` 事件推送。
#[tauri::command]
pub async fn start_audit(
    app: AppHandle,
    options: AuditOptions,
    jobs: State<'_, AuditJobs>,
) -> Result<String, String> {
    let input_dir = PathBuf::from(&options.input_dir);
    if !input_dir.is_dir() {
        return Err("所选论文文件夹不存在或已被移动，请重新选择。".into());
    }
    let config = format_core::rules::load_rule_config(&options.rule_file)
        .map_err(|e| e.user_message())?;
    format_core::rules::validate_rule_config(&config).map_err(|e| e.user_message())?;

    let job_id = next_job_id();
    let token = Arc::new(CancellationToken::new());
    jobs.register(job_id.clone(), token.clone());

    // 预先统计文件数，让前端进度条一开始就有分母。
    let total = format_core::audit::collect_docx_files(&input_dir)
        .map(|v| v.len())
        .unwrap_or(0);

    let app_for_task = app.clone();
    let job_for_task = job_id.clone();
    let annotate = options.generate_annotated_docx;
    tauri::async_runtime::spawn_blocking(move || {
        let forwarder = EventForwarder {
            app: app_for_task.clone(),
            job_id: job_for_task.clone(),
            total,
            completed: Arc::new(Mutex::new(0)),
        };
        match format_core::audit::audit_directory(&input_dir, &config, &forwarder, &token) {
            Ok(mut report) => {
                let outcomes = if annotate { annotate_files(&report) } else { vec![] };
                if !outcomes.is_empty() {
                    let ok = outcomes.iter().filter(|o| o.ok).count();
                    let fail = outcomes.len() - ok;
                    let msg = if fail == 0 {
                        format!("已为 {ok} 个文件生成标注副本（原文件未修改）。")
                    } else {
                        format!("标注副本：成功 {ok} 个，失败 {fail} 个（详见运行详情）。")
                    };
                    let _ = app_for_task.emit(
                        "audit://log",
                        serde_json::json!({
                            "job_id": job_for_task, "at": std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64).unwrap_or(0),
                            "level": if fail == 0 { "info" } else { "warn" },
                            "message": msg,
                        }),
                    );
                    for o in outcomes.iter().filter(|o| !o.ok) {
                        let _ = app_for_task.emit(
                            "audit://log",
                            serde_json::json!({
                                "job_id": job_for_task, "at": std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis() as u64).unwrap_or(0),
                                "level": "warn",
                                "message": o.message,
                            }),
                        );
                    }
                    report.rule_profile = Some(serde_json::to_string(&outcomes).unwrap_or_default());
                }
                let cancelled = token.is_cancelled();
                let _ = app_for_task.emit(
                    "audit://done",
                    serde_json::json!({
                        "job_id": job_for_task,
                        "cancelled": cancelled,
                        "report": report,
                    }),
                );
            }
            Err(e) => {
                eprintln!("[start_audit] {}", e.debug_chain());
                let _ = app_for_task.emit(
                    "audit://error",
                    serde_json::json!({
                        "job_id": job_for_task,
                        "user_message": e.user_message(),
                    }),
                );
            }
        }
        // 无论成败都注销令牌：done/error 已发出，不再有僵尸任务。
        if let Some(state) = app_for_task.try_state::<AuditJobs>() {
            state.finish(&job_for_task);
        }
    });

    Ok(job_id)
}

/// 请求取消审核任务（幂等；剩余文件标记跳过，已完成部分照常出结果）。
#[tauri::command]
pub async fn cancel_audit(job_id: String, jobs: State<'_, AuditJobs>) -> Result<(), String> {
    match jobs.get(&job_id) {
        Some(token) => {
            token.cancel();
            Ok(())
        }
        None => Err("该检查任务已经结束，无需停止。".into()),
    }
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
    let report: format_core::dsl::AuditReport = serde_json::from_str(&report_json)
        .map_err(|e| {
            format!(
                "审核结果数据有误，无法生成报告：{e}\n（请重新运行一次检查后再导出）"
            )
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

#[cfg(test)]
mod tests {
    use super::*;

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
        use std::future::Future;
        use std::task::{Context, Poll, Waker};

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
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
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
