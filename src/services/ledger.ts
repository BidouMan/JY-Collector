import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { Message, Template } from "../types";
import { buildDraftsFromModel, buildLedgerDrafts } from "./extractor";
import { extractRowsWithModel } from "./modelExtract";

export function fileNameFor(template: Template): string {
  const name = template.name.trim() || "默认导出";
  return `${name.replace(/[\/\\:*?"<>|]/g, "_")}.xlsx`;
}

/** 还没设置目标目录时，让用户选一个目录并记回配置。 */
export async function ensureTargetPath(template: Template): Promise<string | null> {
  if (template.targetPath) return template.targetPath;
  const picked = await open({ directory: true, multiple: false });
  const path = Array.isArray(picked) ? picked[0] : picked;
  if (!path) return null;
  const saved = await api.saveTemplate({ ...template, targetPath: path });
  template.targetPath = saved.targetPath;
  return saved.targetPath;
}

export interface GenerateResult {
  ids: string[];
  rows: number;
  engine: "model" | "rule";
  warning: string;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * 优先用模型按列定义抽取；没配 Key、接口失败或个别消息取不到字段时，
 * 用本地规则补齐，并把降级原因交给界面提示。
 */
async function buildDrafts(messages: Message[], template: Template) {
  const ordered = [...messages].sort((a, b) => a.createdAt.localeCompare(b.createdAt));

  if (template.columns.length === 0) {
    return { drafts: buildLedgerDrafts(ordered, template), engine: "rule" as const, warning: "" };
  }

  let apiKey = "";
  try {
    apiKey = (await api.getSettings()).apiKey.trim();
  } catch {
    apiKey = "";
  }
  if (!apiKey) {
    return {
      drafts: buildLedgerDrafts(ordered, template),
      engine: "rule" as const,
      warning: "未配置模型 API Key，已使用本地规则抽取",
    };
  }

  try {
    // 纯图片/文件消息没有正文，模型抽不出内容，不占用 token。
    const extractable = ordered.filter((message) => (message.content ?? "").trim().length > 0);
    const rows = await extractRowsWithModel(extractable, template);
    const { drafts, missing } = buildDraftsFromModel(extractable, rows, template);
    const fallback = missing.length > 0 ? buildLedgerDrafts(missing, template) : [];
    const order = new Map(ordered.map((message, index) => [message.id, index]));
    const merged = [...drafts, ...fallback].sort(
      (a, b) => (order.get(a.messageId) ?? 0) - (order.get(b.messageId) ?? 0),
    );
    return {
      drafts: merged,
      engine: "model" as const,
      warning:
        missing.length > 0
          ? `有 ${missing.length} 条消息模型未取到字段，已按本地规则补齐`
          : "",
    };
  } catch (error) {
    return {
      drafts: buildLedgerDrafts(ordered, template),
      engine: "rule" as const,
      warning: `模型抽取失败，已改用本地规则：${errorText(error)}`,
    };
  }
}

export async function generateRecords(
  messages: Message[],
  template: Template,
): Promise<GenerateResult> {
  const { drafts, engine, warning } = await buildDrafts(messages, template);
  if (!drafts.length) return { ids: [], rows: 0, engine, warning };
  const ids = await api.createRecords(
    drafts.map((draft) => ({
      groupId: draft.groupId,
      title: draft.title,
      data: draft.data,
      sources: draft.sources,
      templateId: template.id,
    })),
  );
  return { ids, rows: drafts.length, engine, warning };
}

export function appendSummary(report: {
  written: number;
  skipped: number;
  addedColumns: string[];
}): string {
  const parts = [`已追加 ${report.written} 行`];
  if (report.skipped > 0) parts.push(`跳过 ${report.skipped} 条已导出`);
  if (report.addedColumns.length > 0) parts.push(`补列：${report.addedColumns.join("、")}`);
  return parts.join("，");
}
