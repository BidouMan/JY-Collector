export type MessageType = "text" | "image" | "file" | "quote";

export interface Group {
  id: string;
  name: string;
  memberCount: number;
  messageCount: number;
  recordCount: number;
  lastAt: string | null;
}

export interface Message {
  id: string;
  groupId: string;
  groupName: string;
  senderId: string;
  senderName: string;
  type: MessageType;
  content: string;
  quotedMessageId: string | null;
  quotedContent: string | null;
  quotedSender: string | null;
  fileName: string | null;
  fileSize: string | null;
  createdAt: string;
}

export interface Avatar {
  entityId: string;
  mime: string;
  /** 图片 base64，不含 data: 前缀 */
  data: string;
}

export interface Person {
  senderId: string;
  senderName: string;
  groupCount: number;
  messageCount: number;
  groupNames: string;
  lastAt: string | null;
}

export interface CaptureScope {
  groups: Group[];
  people: Person[];
  selectedGroups: string[];
  selectedPeople: string[];
  configured: boolean;
}

export type KeywordMode = "fuzzy" | "exact";

export interface SearchQuery {
  keyword: string;
  keywordMode: KeywordMode;
  groupIds: string[];
  senderId: string;
  senderName: string;
  mention: string;
  types: MessageType[];
  startDate: string;
  endDate: string;
  inBasket: boolean | null;
  inRecord: boolean | null;
  sort: "desc" | "asc";
  offset: number;
  limit: number;
}

export interface SearchItem extends Message {
  inBasket: boolean;
  inRecord: boolean;
}

export interface SearchPage {
  items: SearchItem[];
  total: number;
}

export type FieldType = "text" | "number" | "date" | "textarea" | "select";

export interface FieldSchema {
  id: string;
  groupId: string;
  key: string;
  label: string;
  fieldType: FieldType;
  required: boolean;
  example: string;
  sortOrder: number;
}

export interface RecordItem {
  id: string;
  groupId: string;
  groupName: string;
  title: string;
  data: Record<string, string>;
  sourceCount: number;
  templateId: string | null;
  exportedAt: string | null;
  exportBatch: string | null;
  exportTarget: string | null;
  createdAt: string;
  updatedAt: string;
}

export type ColumnFormat = "text" | "number" | "date";

export interface TemplateColumn {
  key: string;
  label: string;
  width: number;
  format: ColumnFormat;
  /** 该列的填写示例，随列定义一起发给模型。 */
  hint: string;
}

export interface Template {
  id: string;
  name: string;
  isDefault: boolean;
  columns: TemplateColumn[];
  targetPath: string;
  sheetName: string;
  headerRow: number;
  fileNameRule: string;
}

export interface AppendReport {
  written: number;
  skipped: number;
  addedColumns: string[];
  path: string;
  batch: string;
  sheet: string;
}

export interface SheetInfo {
  name: string;
  headers: string[];
  rows: number;
}

export interface SheetRowData {
  row: number;
  cells: string[];
}

export interface SheetTable {
  path: string;
  sheet: string;
  headerRow: number;
  headers: string[];
  rows: SheetRowData[];
}

export interface RecordSource {
  fieldKey: string;
  snippet: string;
  message: Message;
}

export interface Settings {
  apiKey: string;
  modelName: string;
  apiBase: string;
  dataDir: string;
}

export interface Bootstrap {
  groups: Group[];
  schemas: FieldSchema[];
}
