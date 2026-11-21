import { api } from "../api";
import type { Message, Template } from "../types";

export interface ModelRow {
  messageId: string;
  values: Record<string, string>;
}

/** 一次请求最多带多少条消息，避免消息太多时超出模型上下文。 */
const CHUNK_SIZE = 20;

/**
 * 调用后端模型抽取：把列定义和消息一起交给模型，按列 key 返回每台设备一行。
 * Key 不经过前端，这里只负责组织请求参数。
 */
export async function extractRowsWithModel(
  messages: Message[],
  template: Template,
): Promise<ModelRow[]> {
  const columns = template.columns.map((column) => ({
    key: column.key,
    label: column.label,
    hint: column.hint,
    format: column.format,
  }));
  // 没有正文的消息（纯图片、文件等）模型也抽不出内容，直接跳过省 token，
  // 这些消息由调用方用本地规则兜底。
  const extractable = messages.filter((message) => (message.content ?? "").trim().length > 0);
  const rows: ModelRow[] = [];

  for (let index = 0; index < extractable.length; index += CHUNK_SIZE) {
    const chunk = extractable.slice(index, index + CHUNK_SIZE);
    const result = await api.extractRows(
      columns,
      chunk.map((message) => ({
        id: message.id,
        content: message.content,
        ...(message.quotedContent
          ? {
              quotedSender: message.quotedSender,
              quotedContent: message.quotedContent,
            }
          : {}),
      })),
    );
    for (const item of result) {
      for (const values of item.rows) {
        rows.push({ messageId: item.messageId, values });
      }
    }
  }

  return rows;
}
