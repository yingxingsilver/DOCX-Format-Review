// P8A lib 层单元测试（vitest，纯 Node 环境）。
// 通过 vi.mock 隔离 Tauri API，验证：错误归一化中文兜底、ApiResult 包装、
// store 状态机流转、事件 job_id 过滤、日志上限、派生 violationsByFile。
import { get } from "svelte/store";
import { beforeEach, describe, expect, it, vi } from "vitest";

// ---- mock Tauri 边界 ----
// 注意：vi.mock 工厂会被 vitest 提升，只能引用 `vi` 本身；
// 所有共享状态统一挂在 vi.fn 的 .mock 对象上（跨模块稳定可达）。
const invokeMock = vi.fn();
const dialogOpenMock = vi.fn();
const openPathMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => (globalThis.__invoke as (...x: unknown[]) => unknown)(...a) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: (e: { payload: unknown }) => void) =>
    (globalThis.__listen as (ev: string, h: (e: { payload: unknown }) => void) => Promise<() => void>)(event, handler),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...a: unknown[]) => (globalThis.__dialog as (...x: unknown[]) => unknown)(...a) }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openPath: (...a: unknown[]) => (globalThis.__openPath as (...x: unknown[]) => unknown)(...a) }));

// 把上面的真实 mock 函数接到 globalThis 通道（工厂内只允许引用 globalThis/vi）。
(globalThis as unknown as Record<string, unknown>).__invoke = invokeMock;
(globalThis as unknown as Record<string, unknown>).__dialog = dialogOpenMock;
(globalThis as unknown as Record<string, unknown>).__openPath = openPathMock;
(globalThis as unknown as Record<string, unknown>).__listen = (event: string, handler: (e: { payload: unknown }) => void) => {
  const handlers = ((invokeMock as unknown as { handlers?: Record<string, (e: { payload: unknown }) => void> }).handlers ??= {});
  handlers[event] = handler;
  return Promise.resolve(() => {
    delete handlers[event];
  });
};

import * as api from "./api";
import * as stores from "./stores";
import type { AuditReport, Location, Violation } from "./types";

function fire(event: string, payload: unknown): void {
  const h = (invokeMock as unknown as { handlers?: Record<string, (e: { payload: unknown }) => void> })
    .handlers?.[event];
  if (!h) throw new Error(`未注册监听: ${event}`);
  h({ payload });
}

const baseLocation: Location = {
  section_index: null,
  block_index: null,
  paragraph_index: null,
  run_index: null,
  page_hint: null,
  object_ref: null,
};

function makeViolation(over: Partial<Violation> = {}): Violation {
  return {
    rule_id: "body-font-size",
    severity: "error",
    file_path: "/tmp/a.docx",
    location: { ...baseLocation, page_hint: 3, paragraph_index: 11 },
    message: "正文字号应为小四（12 磅）",
    expected: "小四（12磅）",
    actual: "五号（10.5磅）",
    excerpt: "示例摘录",
    suggestion: "选中正文调整字号",
    confidence: 0.95,
    requires_manual_review: false,
    ...over,
  };
}

function makeReport(): AuditReport {
  return {
    generated_at: "2026-10-11T00:00:00Z",
    input_dir: "/tmp/papers",
    rule_file: "/tmp/rules.json",
    rule_profile: null,
    total_files: 1,
    passed_files: 0,
    failed_files: 1,
    read_failed_files: 0,
    total_errors: 1,
    total_warnings: 0,
    total_infos: 0,
    files: [
      {
        file_path: "/tmp/papers/a.docx",
        file_name: "a.docx",
        status: "needs_revision",
        error_count: 1,
        warning_count: 0,
        info_count: 0,
        violations: [makeViolation()],
        parse_error: null,
      },
    ],
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  delete (invokeMock as unknown as { handlers?: unknown }).handlers;
  dialogOpenMock.mockReset();
  openPathMock.mockReset();
  stores.disposeAuditEvents();
  stores.resetAll();
});

describe("api.ts 错误归一化", () => {
  it("toUserMessage 保留后端中文消息", () => {
    expect(api.toUserMessage("规则文件不存在，请重新选择。")).toBe("规则文件不存在，请重新选择。");
  });
  it("toUserMessage 对未知异常给中文兜底", () => {
    const msg = api.toUserMessage(new Error(""));
    expect(msg).toContain("请重试");
    expect(/[一-龥]/.test(msg)).toBe(true);
  });
  it("startAudit 失败转 ApiResult.user_message", async () => {
    invokeMock.mockRejectedValue("审核任务启动失败：文件夹不存在");
    const res = await api.startAudit({ input_dir: "/x", rule_file: "/y" });
    expect(res).toEqual({ ok: false, user_message: "审核任务启动失败：文件夹不存在" });
  });
  it("pickInputDir 用户取消返回 data:null 而非错误", async () => {
    dialogOpenMock.mockResolvedValue(null);
    const res = await api.pickInputDir();
    expect(res).toEqual({ ok: true, data: null });
  });
  it("exportReport 序列化 reportJson 并 snake_case 传参", async () => {
    invokeMock.mockResolvedValue("报告已保存到：/tmp/out.html");
    const res = await api.exportReport(makeReport(), "/tmp/out.html", "html");
    expect(res.ok).toBe(true);
    const args = invokeMock.mock.calls[0];
    expect(args[0]).toBe("export_report");
    expect(typeof (args[1] as { reportJson: string }).reportJson).toBe("string");
    expect((args[1] as { format: string }).format).toBe("html");
  });
});

describe("stores.ts 动作与状态机", () => {
  it("startAudit 未选文件夹时中文提示且不进入 running", async () => {
    await stores.startAudit();
    expect(get(stores.status)).toBe("idle");
    expect(get(stores.notice)?.text).toContain("请先选择存放论文");
  });

  it("chooseInputDir 成功后自动刷新列表", async () => {
    dialogOpenMock.mockResolvedValue("/tmp/papers");
    invokeMock.mockResolvedValue([]);
    await stores.chooseInputDir();
    expect(get(stores.inputDir)).toBe("/tmp/papers");
    expect(invokeMock).toHaveBeenCalledWith("list_docx_files", { dir: "/tmp/papers" });
  });

  it("running → done 事件流：summary/status/notice 正确更新", async () => {
    dialogOpenMock.mockResolvedValue("/tmp/papers");
    invokeMock.mockResolvedValue([]); // list_docx_files
    await stores.chooseInputDir();
    dialogOpenMock.mockResolvedValue("/tmp/rules.json");
    await stores.chooseRuleFile();

    invokeMock.mockResolvedValue("job-1"); // start_audit
    await stores.startAudit();
    expect(get(stores.status)).toBe("running");
    expect(get(stores.jobId)).toBe("job-1");

    // 过期 job 的事件必须被忽略（监听已注册，但 handler 按 job_id 过滤）
    fire("audit://done", { job_id: "stale", cancelled: false, report: makeReport() });
    expect(get(stores.summary)).toBeNull();

    fire("audit://progress", {
      job_id: "job-1",
      completed: 1,
      total: 1,
      current_file: "a.docx",
      finished_violations: 1,
    });
    expect(get(stores.progress)).toEqual({ completed: 1, total: 1 });

    fire("audit://log", { job_id: "job-1", at: Date.now(), level: "info", message: "共找到 1 个论文文件" });
    expect(get(stores.logs)).toContain("共找到 1 个论文文件");

    fire("audit://done", { job_id: "job-1", cancelled: false, report: makeReport() });
    expect(get(stores.status)).toBe("done");
    expect(get(stores.summary)?.total_files).toBe(1);
    expect(get(stores.notice)?.kind).toBe("success");
  });

  it("audit://error 置 error 状态并展示中文消息", async () => {
    dialogOpenMock.mockResolvedValue("/tmp/papers");
    invokeMock.mockResolvedValue([]);
    await stores.chooseInputDir();
    dialogOpenMock.mockResolvedValue("/tmp/rules.json");
    await stores.chooseRuleFile();
    invokeMock.mockResolvedValue("job-2");
    await stores.startAudit();
    fire("audit://error", { job_id: "job-2", user_message: "规则文件内容有误，无法开始检查。" });
    expect(get(stores.status)).toBe("error");
    expect(get(stores.notice)?.text).toContain("规则文件内容有误");
  });

  it("violationsByFile 派生 Map 按文件路径索引", () => {
    stores.summary.set(makeReport());
    const map = get(stores.violationsByFile);
    expect(map.get("/tmp/papers/a.docx")?.length).toBe(1);
    expect(map.get("/tmp/papers/a.docx")?.[0].rule_id).toBe("body-font-size");
  });

  it("日志超过上限时截断旧行", () => {
    // pushLog 为内部函数，通过公开事件通道验证：注册监听后灌入 600 条日志。
    invokeMock.mockResolvedValue("job-log");
    dialogOpenMock.mockResolvedValue("/tmp/papers");
    stores.inputDir.set("/tmp/papers");
    stores.rulePath.set("/tmp/rules.json");
    return stores.startAudit().then(() => {
      for (let i = 0; i < 600; i++) {
        fire("audit://log", { job_id: "job-log", at: Date.now(), level: "info", message: `第${i}行` });
      }
      const ls = get(stores.logs);
      expect(ls.length).toBeLessThanOrEqual(500);
      expect(ls[ls.length - 1]).toBe("第599行");
      expect(ls).not.toContain("第0行");
    });
  });

  it("cancelAudit 仅在 running 且有 jobId 时调用后端", async () => {
    await stores.cancelAudit();
    expect(invokeMock).not.toHaveBeenCalledWith("cancel_audit", expect.anything());
  });

  it("exportHtml 无结果时中文提示", async () => {
    await stores.exportHtml();
    expect(get(stores.notice)?.text).toContain("还没有可导出的检查结果");
  });

  it("openReport 拼接默认导出路径", async () => {
    stores.summary.set(makeReport());
    openPathMock.mockResolvedValue(undefined);
    await stores.openReport();
    expect(openPathMock).toHaveBeenCalledWith("/tmp/papers/论文格式检查报告.html");
  });
});

describe("stores.ts 纯格式化函数", () => {
  it("severityLabel 中文映射", () => {
    expect(stores.severityLabel(makeViolation({ severity: "error" }))).toBe("错误");
    expect(stores.severityLabel(makeViolation({ severity: "warning" }))).toBe("警告");
    expect(stores.severityLabel(makeViolation({ severity: "info" }))).toBe("提示");
  });
  it("locationLabel 人话化且段落 1-based", () => {
    expect(stores.locationLabel({ ...baseLocation, page_hint: 3, paragraph_index: 11 })).toBe(
      "大约第 3 页 · 第 12 段",
    );
    expect(stores.locationLabel(baseLocation)).toBe("整篇文档");
  });
});
