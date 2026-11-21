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
    pub group_id: String,
    pub group_name: String,
    pub title: String,
    pub data: serde_json::Value,
    pub source_count: i64,
    pub template_id: Option<String>,
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
}

fn default_header_row() -> i64 {
    1
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetTableDto {
    pub path: String,
    pub sheet: String,
    pub header_row: u32,
    pub headers: Vec<String>,
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

/// sidecar 输出的整体结构
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WxExport {
    pub account: String,
    pub groups: Vec<WxGroupInput>,
    pub messages: Vec<WxMessageInput>,
    #[serde(default)]
    pub avatars: Vec<WxAvatarInput>,
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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapDto {
    pub groups: Vec<GroupDto>,
    pub schemas: Vec<SchemaDto>,
}
