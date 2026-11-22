import { invoke } from "@tauri-apps/api/core";
import type {
  AppendReport,
  Avatar,
  AvatarUrl,
  Bootstrap,
  CaptureScope,
  FieldSchema,
  Group,
  Message,
  Person,
  RecordItem,
  RecordSource,
  SearchPage,
  SearchQuery,
  Settings,
  SheetInfo,
  SheetTable,
  Template,
} from "./types";

export interface RecordSourceInput {
  messageId: string;
  fieldKey: string;
  snippet: string;
}

export interface RecordPayload {
  groupId: string;
  title: string;
  data: Record<string, string>;
  sources: RecordSourceInput[];
  templateId?: string | null;
}

export interface SchemaInput {
  key: string;
  label: string;
  fieldType: string;
  required: boolean;
  example: string;
}

export interface SyncSummary {
  account: string;
  groups: number;
  messages: number;
  added: number;
  durationMs: number;
}

export interface SourceStatus {
  ready: boolean;
  account: string;
  lastError: string;
  lastSyncedAt: string;
  /** 失败原因是口令/密钥问题，界面给出“重新授权”入口。 */
  needsAuth: boolean;
}

/** 首次启动向导 / 重新授权共用的检测结果。 */
export interface SetupStatus {
  platform: string;
  supported: boolean;
  dataFound: boolean;
  account: string;
  accountCount: number;
  keyPresent: boolean;
  authorized: boolean;
  /** ready / needs_auth / no_data / unsupported / error */
  state: "ready" | "needs_auth" | "no_data" | "unsupported" | "error";
  /** 面向用户的一句话状态说明，不含错误码。 */
  message: string;
}

export interface ImagePayload {
  mime: string;
  /** 图片 base64，不含 data: 前缀 */
  data: string;
}

export interface ExtractColumnInput {
  key: string;
  label: string;
  hint: string;
  format: string;
}

export interface ExtractMessageInput {
  id: string;
  content: string;
  quotedSender?: string | null;
  quotedContent?: string | null;
}

export interface ExtractedMessage {
  messageId: string;
  rows: Record<string, string>[];
}

export interface SheetColumnPayload {
  source: number;
  label: string;
  hint: string;
  width: number;
  format: string;
  options?: string[];
}

export const api = {
  bootstrap: () => invoke<Bootstrap>("bootstrap"),

  listGroups: (scoped = false) => invoke<Group[]>("list_groups", { scoped }),

  listMessages: (groupId: string) =>
    invoke<Message[]>("list_messages", { groupId }),

  listPersonMessages: (senderId: string) =>
    invoke<Message[]>("list_person_messages", { senderId }),

  listAvatars: () => invoke<Avatar[]>("list_avatars"),

  listAvatarUrls: () => invoke<AvatarUrl[]>("list_avatar_urls"),

  loadAvatar: (entityId: string) => invoke<ImagePayload>("load_avatar", { entityId }),

  syncAvatars: () => invoke<number>("sync_avatars"),

  loadMessageImage: (md5Hex: string, full = false) =>
    invoke<ImagePayload>("load_message_image", { md5Hex, full }),

  loadMessageEmoticon: (messageId: string) =>
    invoke<ImagePayload>("load_message_emoticon", { messageId }),

  searchPeople: (keyword: string, scoped = false) =>
    invoke<Person[]>("search_people", { keyword, scoped }),

  listFollowed: (keyword = "") => invoke<Person[]>("list_followed", { keyword }),

  toggleFollow: (senderId: string, senderName: string, followed: boolean) =>
    invoke<void>("toggle_follow", { senderId, senderName, followed }),

  getCaptureScope: () => invoke<CaptureScope>("get_capture_scope"),

  saveCaptureScope: (groups: string[], people: string[]) =>
    invoke<void>("save_capture_scope", { groups, people }),

  getMessage: (messageId: string) =>
    invoke<Message | null>("get_message", { messageId }),

  searchMessages: (query: SearchQuery) =>
    invoke<SearchPage>("search_messages", { query }),

  messageContext: (messageId: string, radius = 8) =>
    invoke<Message[]>("message_context", { messageId, radius }),

  listRecords: () => invoke<RecordItem[]>("list_records"),

  createRecord: (payload: RecordPayload) =>
    invoke<string>("create_record", { payload }),

  updateRecord: (id: string, title: string, data: Record<string, string>) =>
    invoke<void>("update_record", { id, title, data }),

  deleteRecord: (id: string) => invoke<void>("delete_record", { id }),

  recordSources: (recordId: string) =>
    invoke<RecordSource[]>("record_sources", { recordId }),

  saveSchemas: (groupId: string, fields: SchemaInput[]) =>
    invoke<FieldSchema[]>("save_schemas", { groupId, fields }),

  getSettings: () => invoke<Settings>("get_settings"),

  saveSetting: (key: string, value: string) =>
    invoke<void>("save_setting", { key, value }),

  createRecords: (payloads: RecordPayload[]) =>
    invoke<string[]>("create_records", { payloads }),

  extractRows: (columns: ExtractColumnInput[], messages: ExtractMessageInput[]) =>
    invoke<ExtractedMessage[]>("extract_rows", { columns, messages }),

  exportRecords: (ids: string[], templateId: string, path: string) =>
    invoke<number>("export_records", { ids, templateId, path }),

  listTemplates: () => invoke<Template[]>("list_templates"),

  readWorkbook: (path: string) => invoke<SheetInfo[]>("read_workbook", { path }),

  importWorkbook: (source: string) =>
    invoke<string>("import_workbook", { source }),

  readSheetData: (templateId: string) =>
    invoke<SheetTable>("read_sheet_data", { templateId }),

  readTemplateSheets: (templateId: string) =>
    invoke<SheetInfo[]>("read_template_sheets", { templateId }),

  readSheetDataNamed: (templateId: string, sheetName: string) =>
    invoke<SheetTable>("read_sheet_data_named", { templateId, sheetName }),

  addTemplateSheet: (templateId: string) =>
    invoke<string>("add_template_sheet", { templateId }),

  renameTemplateSheet: (templateId: string, sheetName: string, newName: string) =>
    invoke<void>("rename_template_sheet", { templateId, sheetName, newName }),

  updateSheetRow: (
    templateId: string,
    sheetName: string,
    row: number,
    values: (string | null)[],
  ) => invoke<void>("update_sheet_row", { templateId, sheetName, row, values }),

  updateSheetRows: (
    templateId: string,
    sheetName: string,
    rows: { row: number; values: (string | null)[] }[],
  ) => invoke<void>("update_sheet_rows", { templateId, sheetName, rows }),

  deleteSheetRow: (templateId: string, sheetName: string, row: number) =>
    invoke<void>("delete_sheet_row", { templateId, sheetName, row }),

  deleteSheetRows: (templateId: string, sheetName: string, rows: number[]) =>
    invoke<void>("delete_sheet_rows", { templateId, sheetName, rows }),

  reorderSheetRows: (templateId: string, sheetName: string, rows: number[]) =>
    invoke<void>("reorder_sheet_rows", { templateId, sheetName, rows }),

  saveSheetColumns: (
    templateId: string,
    sheetName: string,
    columns: SheetColumnPayload[],
  ) => invoke<void>("save_sheet_columns", { templateId, sheetName, columns }),

  appendSheetRow: (templateId: string, sheetName: string, values: string[]) =>
    invoke<number>("append_sheet_row", { templateId, sheetName, values }),

  listWorkbooks: (dir: string) => invoke<string[]>("list_workbooks", { dir }),

  saveTemplate: (input: Template) => invoke<Template>("save_template", { input }),

  deleteTemplate: (id: string) => invoke<void>("delete_template", { id }),

  setDefaultTemplate: (id: string) => invoke<void>("set_default_template", { id }),

  listBasket: () => invoke<Message[]>("list_basket"),

  addToBasket: (ids: string[]) => invoke<void>("add_to_basket", { ids }),

  removeFromBasket: (ids: string[]) => invoke<void>("remove_from_basket", { ids }),

  clearBasket: () => invoke<void>("clear_basket"),

  appendLedger: (
    ids: string[],
    templateId: string,
    path: string,
    includeExported: boolean,
  ) => invoke<AppendReport>("append_ledger", { ids, templateId, path, includeExported }),

  openDataDir: () => invoke<void>("open_data_dir"),

  refreshSource: () => invoke<SyncSummary>("refresh_source"),

  sourceStatus: () => invoke<SourceStatus>("source_status"),

  sourceUpdated: () => invoke<boolean>("source_updated"),

  setupStatus: () => invoke<SetupStatus>("wechat_setup_status"),

  setupCompleted: () => invoke<boolean>("setup_completed"),

  completeSetup: () => invoke<void>("complete_setup"),
};
