export type MessageType = "text" | "image" | "file" | "quote" | "emoticon";

export interface Group {
  id: string;
  name: string;
  memberCount: number;
  messageCount: number;
  recordCount: number;
  lastAt: string | null;
  /** group：群聊；private：单聊 */
  kind: "group" | "private";
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
  /** 图片消息对应的本地文件 md5，用于按需解密预览 */
  imageMd5: string | null;
  /** 表情消息 md5，用于按需下载并缓存 */
  emoticonMd5: string | null;
  createdAt: string;
}

export interface Avatar {
  entityId: string;
  mime: string;
  /** 图片 base64，不含 data: 前缀 */
  data: string;
}

/** 本地没有头像缓存时的 CDN 地址 */
export interface AvatarUrl {
  entityId: string;
  url: string;
}

export interface Person {
  senderId: string;
  senderName: string;
  groupCount: number;
  messageCount: number;
  groupNames: string;
  lastAt: string | null;
  isFollowed: boolean;
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

export type ColumnFormat =
  | "text"
  | "number"
  | "date"
  | "select"
  | "multi_select";

export interface TemplateColumn {
  key: string;
  label: string;
  width: number;
  format: ColumnFormat;
  /** 该列的填写示例，随列定义一起发给模型。 */
  hint: string;
  /** 单选/多选列的候选项。 */
  options?: string[];
}

/** 模板里的一个工作表。 */
export interface TemplateSheet {
  name: string;
  headerRow: number;
  columns: TemplateColumn[];
  /** 录入和导出默认使用的工作表。 */
  primary: boolean;
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
  /** 多工作表定义；为空时按 sheetName + columns 的单表结构处理。 */
  sheets: TemplateSheet[];
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
  /** 与 headers 一一对应的模板列 key，用于视图配置稳定引用列。 */
  keys: string[];
  rows: SheetRowData[];
}

/** 表格视图配置：只影响展示，不改动 Excel 数据。 */
export interface TableViewFilter {
  key: string;
  operator: "contains" | "equals" | "empty" | "notEmpty" | "gt" | "lt";
  value: string;
}

export interface TableViewSort {
  key: string;
  direction: "asc" | "desc";
}

export interface TableViewGroup {
  key: string;
  direction: "asc" | "desc";
}

export interface TableViewConfig {
  filters?: TableViewFilter[];
  /** 多个筛选条件之间的关系：and=全部满足，or=任一满足。 */
  filterLogic?: "and" | "or";
  sorts?: TableViewSort[];
  groupKeys?: TableViewGroup[];
  hiddenKeys?: string[];
  /** 冻结第一个可见列。 */
  freezeFirstColumn?: boolean;
  /** @deprecated 兼容旧本地缓存 */
  sort?: TableViewSort | null;
  /** @deprecated 兼容旧本地缓存 */
  groupKey?: string;
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
