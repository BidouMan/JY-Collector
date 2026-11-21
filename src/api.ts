import { invoke } from "@tauri-apps/api/core";
import type {
  AppendReport,
  Avatar,
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

export const api = {
  bootstrap: () => invoke<Bootstrap>("bootstrap"),

  listGroups: (scoped = false) => invoke<Group[]>("list_groups", { scoped }),

  listMessages: (groupId: string) =>
    invoke<Message[]>("list_messages", { groupId }),

  listPersonMessages: (senderId: string) =>
    invoke<Message[]>("list_person_messages", { senderId }),

  listAvatars: () => invoke<Avatar[]>("list_avatars"),

  syncAvatars: () => invoke<number>("sync_avatars"),

  searchPeople: (keyword: string, scoped = false) =>
    invoke<Person[]>("search_people", { keyword, scoped }),

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

  readSheetData: (templateId: string) =>
    invoke<SheetTable>("read_sheet_data", { templateId }),

  updateSheetRow: (templateId: string, row: number, values: (string | null)[]) =>
    invoke<void>("update_sheet_row", { templateId, row, values }),

  deleteSheetRow: (templateId: string, row: number) =>
    invoke<void>("delete_sheet_row", { templateId, row }),

  appendSheetRow: (templateId: string, values: string[]) =>
    invoke<number>("append_sheet_row", { templateId, values }),

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
};
