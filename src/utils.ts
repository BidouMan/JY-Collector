import type { MessageType } from "./types";

export const TYPE_LABEL: Record<MessageType, string> = {
  text: "文本",
  quote: "引用",
  image: "图片",
  file: "文件",
};

function localDate(offsetDays = 0): string {
  const date = new Date();
  date.setDate(date.getDate() + offsetDays);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

export function formatTime(value: string | null): string {
  if (!value) return "";
  const date = value.slice(0, 10);
  const clock = value.slice(11, 16);
  if (date === localDate(0)) return `今天 ${clock}`;
  if (date === localDate(-1)) return `昨天 ${clock}`;
  return `${value.slice(5, 10)} ${clock}`;
}

export function formatDay(value: string | null): string {
  if (!value) return "";
  return value.slice(0, 10);
}

export interface TextPart {
  text: string;
  mark: boolean;
}

export function splitHighlight(text: string, snippet?: string | null): TextPart[] {
  if (!snippet) return [{ text, mark: false }];
  const index = text.indexOf(snippet);
  if (index < 0) return [{ text, mark: false }];
  const parts: TextPart[] = [];
  if (index > 0) parts.push({ text: text.slice(0, index), mark: false });
  parts.push({ text: snippet, mark: true });
  const end = index + snippet.length;
  if (end < text.length) parts.push({ text: text.slice(end), mark: false });
  return parts;
}

export function titleFor(data: Record<string, string>, labels: string[]): string {
  const values = labels
    .map((label) => data[label]?.trim())
    .filter((value) => value);
  if (values.length > 0) return values.slice(0, 2).join(" · ");
  return "未命名记录";
}
