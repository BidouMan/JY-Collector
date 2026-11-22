use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDto {
    pub id: String,
    pub name: String,
    pub member_count: i64,
    pub message_count: i64,
    pub record_count: i64,
    pub last_at: Option<String>,
    /// 会话类型：group（群聊）或 private（单聊）
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDto {
    pub id: String,
    pub group_id: String,
    pub group_name: String,
    pub sender_id: String,
    pub sender_name: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub content: String,
    pub quoted_message_id: Option<String>,
    pub quoted_content: Option<String>,
    pub quoted_sender: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<String>,
    /// 图片消息对应的本地 .dat 文件 md5（非图片为空）
    pub image_md5: Option<String>,
    pub emoticon_md5: Option<String>,
    pub emoticon_url: Option<String>,
    pub emoticon_key: Option<String>,
    pub created_at: String,
}

/// 设置页「采集范围」：全部可选项 + 已勾选项。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureScopeDto {
    pub groups: Vec<GroupDto>,
    pub people: Vec<PersonDto>,
    pub selected_groups: Vec<String>,
    pub selected_people: Vec<String>,
    /// 是否已经保存过采集范围；未保存时默认全部采集。
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonDto {
    pub sender_id: String,
    pub sender_name: String,
    pub group_count: i64,
    pub message_count: i64,
    pub group_names: String,
    pub last_at: Option<String>,
    /// 是否已加入关注
    pub is_followed: bool,
}

/// 高级检索条件。字段留空表示不限制。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    #[serde(default)]
    pub keyword: String,
    /// fuzzy（包含） | exact（整段相同）
    #[serde(default)]
    pub keyword_mode: String,
    #[serde(default)]
    pub group_ids: Vec<String>,
    #[serde(default)]
    pub sender_id: String,
    #[serde(default)]
    pub sender_name: String,
    #[serde(default)]
    pub mention: String,
    #[serde(default)]
    pub types: Vec<String>,
    /// YYYY-MM-DD
    #[serde(default)]
    pub start_date: String,
    #[serde(default)]
    pub end_date: String,
    #[serde(default)]
    pub in_basket: Option<bool>,
    #[serde(default)]
    pub in_record: Option<bool>,
    /// desc（新到旧） | asc（旧到新）
    #[serde(default)]
    pub sort: String,
    #[serde(default)]
    pub offset: i64,
    #[serde(default)]
    pub limit: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchItemDto {
    #[serde(flatten)]
    pub message: MessageDto,
    pub in_basket: bool,
    pub in_record: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPageDto {
    pub items: Vec<SearchItemDto>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDto {
    pub id: String,
    pub group_id: String,
    pub key: String,
    pub label: String,
    pub field_type: String,
    pub required: bool,
    pub example: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordDto {
    pub id: String,
    pub group_id: Option<String>,
    pub group_name: String,
    pub title: String,
    pub data: serde_json::Value,
    pub source_count: i64,
    pub template_id: Option<String>,
    /// 所属工作表（台账行从这里区分 sheet）。
    pub sheet_name: String,
    /// 工作表内的行顺序，拖拽排序只改这个值。
    pub sort_order: f64,
    /// capture（采集录入）| manual（手工新增）| import（Excel 导入）
    pub origin: String,
    pub exported_at: Option<String>,
    pub export_batch: Option<String>,
    pub export_target: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordSourceDto {
    pub field_key: String,
    pub snippet: String,
    pub message: MessageDto,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaInput {
    pub key: String,
    pub label: String,
    pub field_type: String,
    pub required: bool,
    pub example: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordSourceInput {
    pub message_id: String,
    pub field_key: String,
    pub snippet: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordPayload {
    pub group_id: String,
    pub title: String,
    pub data: serde_json::Value,
    pub sources: Vec<RecordSourceInput>,
    #[serde(default)]
    pub template_id: Option<String>,
}

/// 台账模板的一列。key 用于记录动态字段，label 用于 Excel 表头匹配。
/// hint 是该列的填写示例（随列定义一起发给模型），可为空。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateColumn {
    pub key: String,
    pub label: String,
    #[serde(default = "default_column_width")]
    pub width: u32,
    #[serde(default = "default_column_format")]
    pub format: String,
    #[serde(default)]
    pub hint: String,
    /// 单选/多选列的候选项。
    #[serde(default)]
    pub options: Vec<String>,
}

/// 模板里的一个工作表：名称、表头行和列定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSheet {
    pub name: String,
    #[serde(default = "default_header_row")]
    pub header_row: i64,
    #[serde(default)]
    pub columns: Vec<TemplateColumn>,
    /// 录入和导出默认使用的工作表。
    #[serde(default)]
    pub primary: bool,
}

fn default_column_width() -> u32 {
    18
}

fn default_column_format() -> String {
    "text".to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateDto {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub columns: Vec<TemplateColumn>,
    pub target_path: String,
    pub sheet_name: String,
    pub header_row: i64,
    pub file_name_rule: String,
    /// 多工作表定义；空表示沿用 sheet_name/header_row/columns 的单表结构。
    pub sheets: Vec<TemplateSheet>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub columns: Vec<TemplateColumn>,
    #[serde(default)]
    pub target_path: String,
    #[serde(default)]
    pub sheet_name: String,
    #[serde(default = "default_header_row")]
    pub header_row: i64,
    #[serde(default)]
    pub file_name_rule: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub sheets: Vec<TemplateSheet>,
}

fn default_header_row() -> i64 {
    1
}

/// 数据管理页「列设置」保存时的一列。
/// `source` 是这一列在调整前的 Excel 列号；0 表示新增列。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetColumnInput {
    #[serde(default)]
    pub source: u32,
    pub label: String,
    #[serde(default)]
    pub hint: String,
    #[serde(default = "default_column_width")]
    pub width: u32,
    #[serde(default = "default_column_format")]
    pub format: String,
    #[serde(default)]
    pub options: Vec<String>,
}

/// 批量写单元格：`values` 与列一一对应，null 表示保持原值。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetRowUpdate {
    pub row: u32,
    pub values: Vec<Option<String>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendReportDto {
    pub written: i64,
    pub skipped: i64,
    pub added_columns: Vec<String>,
    pub path: String,
    pub batch: String,
    pub sheet: String,
}

/// 单个工作表的对账结果。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerImportSheetDto {
    pub name: String,
    pub excel_rows: i64,
    pub matched: i64,
    pub excel_only: i64,
    pub record_only: i64,
    pub updated: i64,
}

/// 只读对账预览：先看数字，确认后再落库。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerImportPreviewDto {
    pub template_id: String,
    pub workbook: String,
    pub sheets: Vec<LedgerImportSheetDto>,
    pub total_excel_rows: i64,
    pub total_matched: i64,
    pub total_excel_only: i64,
    pub total_record_only: i64,
    pub total_updated: i64,
    pub columns_added: Vec<String>,
    pub ignored_sheets: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerImportReportDto {
    pub preview: LedgerImportPreviewDto,
    pub inserted: i64,
    pub updated: i64,
    pub policy: String,
    pub backup_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetInfoDto {
    pub name: String,
    pub headers: Vec<String>,
    pub rows: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetRowDto {
    pub row: u32,
    pub cells: Vec<String>,
}

/// 数据库台账行：id 是稳定身份，row 只用于界面展示。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerRowDto {
    pub id: String,
    pub row: u32,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerTableDto {
    pub template_id: String,
    pub sheet: String,
    pub header_row: u32,
    pub headers: Vec<String>,
    pub keys: Vec<String>,
    pub rows: Vec<LedgerRowDto>,
}

/// 批量写行：`values` 与模板列一一对应，null 表示保留原值。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerRowUpdate {
    pub id: String,
    pub values: Vec<Option<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetTableDto {
    pub path: String,
    pub sheet: String,
    pub header_row: u32,
    pub headers: Vec<String>,
    /// 与 headers 一一对应的模板列 key，用于筛选/排序等视图配置稳定引用列。
    pub keys: Vec<String>,
    pub rows: Vec<SheetRowDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub api_key: String,
    pub model_name: String,
    pub api_base: String,
    pub data_dir: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WxGroupInput {
    pub id: String,
    pub name: String,
    pub member_count: i64,
    /// Unix 秒；没有活跃记录时为 null
    pub last_active: Option<i64>,
    /// sidecar 未提供时按群聊处理
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WxMessageInput {
    pub group_id: String,
    pub local_id: i64,
    pub sender_id: String,
    pub sender_name: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub content: Option<String>,
    pub quoted_sender: Option<String>,
    pub quoted_content: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<String>,
    #[serde(default)]
    pub image_md5: Option<String>,
    #[serde(default)]
    pub emoticon_md5: Option<String>,
    #[serde(default)]
    pub emoticon_url: Option<String>,
    #[serde(default)]
    pub emoticon_key: Option<String>,
    /// Unix 秒
    pub created_at: i64,
}

/// sidecar 输出的头像：data 为图片的 base64（不含 data: 前缀）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WxAvatarInput {
    pub entity_id: String,
    pub mime: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarDto {
    pub entity_id: String,
    pub mime: String,
    pub data: String,
}

/// 没有本地头像缓存时的 CDN 地址（群成员也能覆盖到）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WxAvatarUrlInput {
    pub entity_id: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarUrlDto {
    pub entity_id: String,
    pub url: String,
}

/// 图片消息解码结果：data 为图片 base64（不含 data: 前缀）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePayloadDto {
    pub mime: String,
    pub data: String,
}

/// sidecar 输出的整体结构
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WxExport {
    pub account: String,
    pub groups: Vec<WxGroupInput>,
    pub messages: Vec<WxMessageInput>,
    #[serde(default)]
    pub avatars: Vec<WxAvatarInput>,
    #[serde(default)]
    pub avatar_urls: Vec<WxAvatarUrlInput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummaryDto {
    pub account: String,
    pub groups: i64,
    pub messages: i64,
    pub added: i64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatusDto {
    pub ready: bool,
    pub account: String,
    pub last_error: String,
    pub last_synced_at: String,
    /// 失败原因是口令/密钥问题，界面据此给出“重新授权”入口。
    pub needs_auth: bool,
}

/// 首次启动向导与重新授权入口共用的数据源检测结果。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatusDto {
    pub platform: String,
    pub supported: bool,
    pub data_found: bool,
    pub account: String,
    pub account_count: i64,
    pub key_present: bool,
    pub authorized: bool,
    /// ready / needs_auth / no_data / unsupported / error
    pub state: String,
    /// 面向用户的一句话状态说明，不包含错误码。
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapDto {
    pub groups: Vec<GroupDto>,
    pub schemas: Vec<SchemaDto>,
}
