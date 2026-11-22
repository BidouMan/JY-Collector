mod db;
mod export;
mod extract;
mod ledger;
mod ledger_import;
mod ledger_store;
mod media;
mod models;
mod wechat;

use models::{
    AppendReportDto, AvatarDto, AvatarUrlDto, BootstrapDto, CaptureScopeDto, GroupDto,
    ImagePayloadDto, LedgerImportPreviewDto, LedgerImportReportDto, LedgerImportSheetDto,
    LedgerRowUpdate, LedgerTableDto,
    MessageDto, PersonDto, RecordDto, RecordPayload, RecordSourceDto, SchemaDto, SchemaInput,
    SearchPageDto, SearchQuery, SettingsDto, SetupStatusDto, SheetColumnInput, SheetInfoDto,
    SheetRowDto, SheetRowUpdate, SheetTableDto, SourceStatusDto, SyncSummaryDto,
    TemplateColumn, TemplateDto, TemplateInput, TemplateSheet,
};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, State};

struct DbState(Mutex<rusqlite::Connection>);
struct DataDirState(std::path::PathBuf);

fn db_err(message: impl ToString) -> String {
    message.to_string()
}

/// sidecar 目录：优先环境变量，其次打包资源目录，最后是开发期的 src-tauri/sidecar。
fn resolve_sidecar_dir(app: &tauri::AppHandle) -> std::path::PathBuf {
    if let Ok(path) = std::env::var("JY_SIDECAR_DIR") {
        let dir = std::path::PathBuf::from(path);
        if dir.join("wechat_export.py").is_file() {
            return dir;
        }
    }
    if let Ok(resource) = app.path().resource_dir() {
        let dir = resource.join("sidecar");
        if dir.join("wechat_export.py").is_file() {
            return dir;
        }
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sidecar")
}

fn record_sync_result(source: &wechat::SourceState, result: &Result<SyncSummaryDto, String>) {
    // 先更新状态再释放锁，避免与 has_update 形成锁顺序交叉。
    let synced_account = match result {
        Ok(summary) => {
            if let Ok(mut status) = source.status.lock() {
                status.ready = true;
                status.account = summary.account.clone();
                status.last_error = String::new();
                status.needs_auth = false;
                status.last_synced_at = chrono::Local::now()
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string();
            }
            Some(summary.account.clone())
        }
        Err(message) => {
            if let Ok(mut status) = source.status.lock() {
                status.last_error = message.clone();
                status.needs_auth = wechat::is_auth_error(message);
            }
            None
        }
    };
    if let Some(account) = synced_account {
        wechat::mark_synced(source, &account);
    }
}

#[tauri::command]
fn bootstrap(db: State<DbState>) -> Result<BootstrapDto, String> {
    let conn = db.0.lock().map_err(db_err)?;
    Ok(BootstrapDto {
        groups: db::list_groups(&conn).map_err(db_err)?,
        schemas: db::list_all_schemas(&conn).map_err(db_err)?,
    })
}

#[tauri::command]
fn list_groups(scoped: Option<bool>, db: State<DbState>) -> Result<Vec<GroupDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    if scoped.unwrap_or(false) {
        db::list_groups_scoped(&conn).map_err(db_err)
    } else {
        db::list_groups(&conn).map_err(db_err)
    }
}

#[tauri::command]
fn list_messages(group_id: String, db: State<DbState>) -> Result<Vec<MessageDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::list_messages(&conn, &group_id).map_err(db_err)
}

#[tauri::command]
fn list_person_messages(sender_id: String, db: State<DbState>) -> Result<Vec<MessageDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::list_person_messages(&conn, &sender_id).map_err(db_err)
}

#[tauri::command]
fn list_avatars(db: State<DbState>) -> Result<Vec<AvatarDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::list_avatars(&conn).map_err(db_err)
}

/// 只同步头像：不想重解消息库时用于首次补齐头像，失败不影响主流程。
#[tauri::command]
fn sync_avatars(
    db: State<DbState>,
    source: State<wechat::SourceState>,
) -> Result<usize, String> {
    let account = source.status.lock().map_err(db_err)?.account.clone();
    let snapshot = wechat::avatar_snapshot(&source.sidecar_dir, &account)?;
    let count = snapshot.avatars.len();
    let mut conn = db.0.lock().map_err(db_err)?;
    db::apply_avatars(&mut conn, &snapshot.avatars, &snapshot.urls).map_err(db_err)?;
    Ok(count)
}

#[tauri::command]
fn list_avatar_urls(db: State<DbState>) -> Result<Vec<AvatarUrlDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::list_avatar_urls(&conn).map_err(db_err)
}

/// 按需下载联系人头像（群成员也能覆盖到），不落盘。
#[tauri::command]
async fn load_avatar(
    entity_id: String,
    db: State<'_, DbState>,
) -> Result<ImagePayloadDto, String> {
    let url = {
        let conn = db.0.lock().map_err(db_err)?;
        db::avatar_url(&conn, &entity_id).map_err(db_err)?
    };
    let url = url.ok_or_else(|| "没有可用的头像地址".to_string())?;
    let (mime, data) = media::load_avatar(&url).await?;
    Ok(ImagePayloadDto { mime, data })
}

/// 按需下载并解密一条表情消息（不落盘）。
#[tauri::command]
async fn load_message_emoticon(
    message_id: String,
    db: State<'_, DbState>,
) -> Result<ImagePayloadDto, String> {
    let (_, url, key) = {
        let conn = db.0.lock().map_err(db_err)?;
        db::message_emoticon(&conn, &message_id)
            .map_err(db_err)?
            .ok_or_else(|| "这条表情没有可用的下载地址".to_string())?
    };
    let (mime, data) = media::load_emoticon(&url, key.as_deref()).await?;
    Ok(ImagePayloadDto { mime, data })
}

/// 按需解密一条图片消息。full=false 取缩略图，true 取原图。
#[tauri::command]
fn load_message_image(
    md5_hex: String,
    full: bool,
    source: State<wechat::SourceState>,
) -> Result<ImagePayloadDto, String> {
    let account = source.status.lock().map_err(db_err)?.account.clone();
    if account.is_empty() {
        return Err("尚未同步微信数据".to_string());
    }
    let (mime, data) = media::load_image(&account, &md5_hex, full)?;
    Ok(ImagePayloadDto { mime, data })
}

#[tauri::command]
fn search_people(
    keyword: String,
    scoped: Option<bool>,
    db: State<DbState>,
) -> Result<Vec<PersonDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::search_people(&conn, &keyword, scoped.unwrap_or(false)).map_err(db_err)
}

#[tauri::command]
fn get_message(message_id: String, db: State<DbState>) -> Result<Option<MessageDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::get_message(&conn, &message_id).map_err(db_err)
}

#[tauri::command]
fn list_followed(keyword: String, db: State<DbState>) -> Result<Vec<PersonDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::list_followed_people(&conn, &keyword).map_err(db_err)
}

#[tauri::command]
fn toggle_follow(
    sender_id: String,
    sender_name: String,
    followed: bool,
    db: State<DbState>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::set_followed(&conn, &sender_id, &sender_name, followed).map_err(db_err)
}

#[tauri::command]
fn search_messages(query: SearchQuery, db: State<DbState>) -> Result<SearchPageDto, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::search_messages(&conn, &query).map_err(db_err)
}

#[tauri::command]
fn message_context(
    message_id: String,
    radius: Option<i64>,
    db: State<DbState>,
) -> Result<Vec<MessageDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::message_context(&conn, &message_id, radius.unwrap_or(8)).map_err(db_err)
}

/// 用模型按列定义抽取台账字段；Key 只在后端读取，不经过前端。
#[tauri::command]
async fn extract_rows(
    columns: Vec<extract::ExtractColumnInput>,
    messages: Vec<extract::ExtractMessageInput>,
    db: State<'_, DbState>,
) -> Result<Vec<extract::ExtractedMessage>, String> {
    let (api_key, model, base) = {
        let conn = db.0.lock().map_err(db_err)?;
        let settings = db::get_settings(&conn, "").map_err(db_err)?;
        (settings.api_key, settings.model_name, settings.api_base)
    };
    extract::call_model(&base, &api_key, &model, &columns, &messages).await
}

#[tauri::command]
fn list_records(db: State<DbState>) -> Result<Vec<RecordDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::list_records(&conn).map_err(db_err)
}

#[tauri::command]
fn create_record(payload: RecordPayload, db: State<DbState>) -> Result<String, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    db::create_record(&mut conn, &payload).map_err(db_err)
}

#[tauri::command]
fn update_record(
    id: String,
    title: String,
    data: serde_json::Value,
    db: State<DbState>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::update_record(&conn, &id, &title, &data).map_err(db_err)
}

#[tauri::command]
fn delete_record(id: String, db: State<DbState>) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::delete_record(&conn, &id).map_err(db_err)
}

#[tauri::command]
fn record_sources(record_id: String, db: State<DbState>) -> Result<Vec<RecordSourceDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::list_record_sources(&conn, &record_id).map_err(db_err)
}

#[tauri::command]
fn save_schemas(
    group_id: String,
    fields: Vec<SchemaInput>,
    db: State<DbState>,
) -> Result<Vec<SchemaDto>, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    db::save_schemas(&mut conn, &group_id, &fields).map_err(db_err)
}

#[tauri::command]
fn get_settings(
    db: State<DbState>,
    data_dir: State<DataDirState>,
) -> Result<SettingsDto, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::get_settings(&conn, &data_dir.0.to_string_lossy()).map_err(db_err)
}

#[tauri::command]
fn save_setting(key: String, value: String, db: State<DbState>) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::save_setting(&conn, &key, &value).map_err(db_err)
}

/// 设置页「采集范围」：返回全部群组/联系人与当前勾选情况。
#[tauri::command]
fn get_capture_scope(db: State<DbState>) -> Result<CaptureScopeDto, String> {
    let conn = db.0.lock().map_err(db_err)?;
    let (selected_groups, selected_people, configured) =
        db::capture_scope(&conn).map_err(db_err)?;
    Ok(CaptureScopeDto {
        groups: db::list_groups(&conn).map_err(db_err)?,
        people: db::search_people(&conn, "", false).map_err(db_err)?,
        selected_groups,
        selected_people,
        configured,
    })
}

#[tauri::command]
fn save_capture_scope(
    groups: Vec<String>,
    people: Vec<String>,
    db: State<DbState>,
) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    db::save_capture_scope(&mut conn, &groups, &people).map_err(db_err)
}

#[tauri::command]
fn create_records(payloads: Vec<RecordPayload>, db: State<DbState>) -> Result<Vec<String>, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let mut ids = Vec::with_capacity(payloads.len());
    for payload in &payloads {
        ids.push(db::create_record(&mut conn, payload).map_err(db_err)?);
    }
    Ok(ids)
}

#[tauri::command]
fn export_records(
    ids: Vec<String>,
    template_id: String,
    path: String,
    db: State<DbState>,
) -> Result<usize, String> {
    let conn = db.0.lock().map_err(db_err)?;
    export::export_records(&conn, &ids, &template_id, &path)
}

#[tauri::command]
fn list_templates(db: State<DbState>) -> Result<Vec<TemplateDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::list_templates(&conn).map_err(db_err)
}

fn export_file_name(template: &TemplateDto) -> String {
    let name = template.name.trim();
    let base: String = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    let base = if base.is_empty() {
        "默认导出".to_string()
    } else {
        base
    };
    format!("{}.xlsx", base)
}

/// 目标路径可能是目录（新语义）或旧的完整文件路径，统一解析成 xlsx 文件。
fn resolve_export_path(template: &TemplateDto) -> Option<PathBuf> {
    let raw = template.target_path.trim();
    if raw.is_empty() {
        return None;
    }
    let direct = PathBuf::from(raw);
    if raw.to_lowercase().ends_with(".xlsx") {
        return Some(direct);
    }
    Some(direct.join(export_file_name(template)))
}

#[tauri::command]
fn list_workbooks(dir: String) -> Result<Vec<String>, String> {
    let path = PathBuf::from(dir.trim());
    let mut names: Vec<String> = Vec::new();
    let entries = match std::fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(_) => return Ok(names),
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("~$") {
            continue;
        }
        if name.to_lowercase().ends_with(".xlsx") {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// 把客户上传的 Excel 复制到应用数据目录，返回副本路径。
#[tauri::command]
fn import_workbook(source: String, data_dir: State<DataDirState>) -> Result<String, String> {
    let src = PathBuf::from(source.trim());
    if !src.is_file() {
        return Err("找不到要上传的 Excel 文件".to_string());
    }
    let is_xlsx = src
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("xlsx"))
        .unwrap_or(false);
    if !is_xlsx {
        return Err("只支持 .xlsx 文件".to_string());
    }
    let dir = data_dir.0.join("templates");
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建模板目录：{}", e))?;
    let stem = src
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("模板");
    let mut target = dir.join(format!("{}.xlsx", stem));
    let mut suffix = 2;
    while target.exists() {
        target = dir.join(format!("{} ({}).xlsx", stem, suffix));
        suffix += 1;
    }
    std::fs::copy(&src, &target).map_err(|e| format!("复制 Excel 失败：{}", e))?;
    Ok(target.to_string_lossy().to_string())
}

#[tauri::command]
fn read_workbook(path: String) -> Result<Vec<SheetInfoDto>, String> {
    let mut file = PathBuf::from(path.trim());
    if file.is_dir() {
        file = file.join("默认导出.xlsx");
    }
    let sheets = ledger::read_workbook(&file)?;
    Ok(sheets
        .into_iter()
        .map(|sheet| SheetInfoDto {
            name: sheet.name,
            headers: sheet.headers,
            rows: sheet.rows,
        })
        .collect())
}

fn resolve_template(
    conn: &rusqlite::Connection,
    template_id: &str,
) -> Result<TemplateDto, String> {
    db::get_template(conn, template_id)
        .map_err(db_err)?
        .or_else(|| db::list_templates(conn).ok()?.into_iter().next())
        .ok_or_else(|| "没有可用的导出配置".to_string())
}

fn resolve_sheet_name(template: &TemplateDto, sheet_name: Option<String>) -> String {
    sheet_name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            if template.sheet_name.trim().is_empty() {
                "Sheet1".to_string()
            } else {
                template.sheet_name.clone()
            }
        })
}

fn sheet_header_row(template: &TemplateDto, sheet_name: &str) -> u32 {
    template
        .sheets
        .iter()
        .find(|sheet| sheet.name == sheet_name)
        .map(|sheet| sheet.header_row)
        .unwrap_or(template.header_row)
        .max(1) as u32
}

pub(crate) fn sheet_columns(template: &TemplateDto, sheet_name: &str) -> Vec<TemplateColumn> {
    template
        .sheets
        .iter()
        .find(|sheet| sheet.name == sheet_name)
        .map(|sheet| sheet.columns.clone())
        .unwrap_or_else(|| template.columns.clone())
}

/// 让列 key 与表头数量对齐，缺失的列给一个稳定兜底 key。
fn column_keys(columns: &[TemplateColumn], count: usize) -> Vec<String> {
    (0..count)
        .map(|index| {
            columns
                .get(index)
                .map(|column| column.key.clone())
                .unwrap_or_else(|| format!("col_{}", index + 1))
        })
        .collect()
}

fn column_cell_value(
    columns: &[TemplateColumn],
    index: usize,
    text: &str,
) -> ledger::CellValue {
    let is_number = columns
        .get(index)
        .map(|column| column.format == "number")
        .unwrap_or(false);
    if is_number {
        if let Ok(number) = text.trim().parse::<f64>() {
            return ledger::CellValue::Number(number);
        }
    }
    ledger::CellValue::Text(text.to_string())
}

/// 表格列可能比配置多（例如手动导入的表格），先补齐配置列，保证按列号同步时不会错位。
pub(crate) fn pad_template_columns(columns: &mut Vec<TemplateColumn>, headers: &[String]) {
    for (index, label) in headers.iter().enumerate() {
        if index >= columns.len() {
            let key = next_column_key(columns, index);
            columns.push(TemplateColumn {
                key,
                label: label.clone(),
                width: 18,
                format: "text".to_string(),
                hint: String::new(),
                options: Vec::new(),
            });
        }
    }
}

fn next_column_key(columns: &[TemplateColumn], start: usize) -> String {
    let mut counter = start + 1;
    loop {
        let key = format!("col_{}", counter);
        if !columns.iter().any(|column| column.key == key) {
            return key;
        }
        counter += 1;
    }
}

pub(crate) fn next_column_key_from(
    used: &std::collections::HashSet<String>,
    start: usize,
) -> String {
    let mut counter = start + 1;
    loop {
        let key = format!("col_{}", counter);
        if !used.contains(&key) {
            return key;
        }
        counter += 1;
    }
}

fn save_sheet_columns_for(
    conn: &mut rusqlite::Connection,
    template: &TemplateDto,
    sheet_name: &str,
    columns: Vec<TemplateColumn>,
) -> Result<(), String> {
    let mut sheets = template.sheets.clone();
    let index = sheets
        .iter()
        .position(|sheet| sheet.name == sheet_name)
        .ok_or_else(|| format!("模板里没有工作表「{}」", sheet_name))?;
    sheets[index].columns = columns;

    let primary = sheets
        .iter()
        .find(|sheet| sheet.primary)
        .or_else(|| sheets.first())
        .cloned()
        .ok_or_else(|| "模板没有可用的工作表".to_string())?;
    let input = TemplateInput {
        id: Some(template.id.clone()),
        name: template.name.clone(),
        columns: primary.columns.clone(),
        target_path: template.target_path.clone(),
        sheet_name: primary.name.clone(),
        header_row: primary.header_row,
        file_name_rule: template.file_name_rule.clone(),
        is_default: template.is_default,
        sheets,
    };
    db::save_template(conn, &input).map_err(db_err)?;
    Ok(())
}

fn save_template_columns(
    conn: &mut rusqlite::Connection,
    template: &TemplateDto,
    columns: Vec<TemplateColumn>,
) -> Result<(), String> {
    let sheet_name = template.sheet_name.clone();
    save_sheet_columns_for(conn, template, &sheet_name, columns)
}

/// 读取最终 Excel 表格的完整内容；文件还没生成时按模板列返回空表。
#[tauri::command]
fn read_sheet_data(template_id: String, db: State<DbState>) -> Result<SheetTableDto, String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let sheet_name = if template.sheet_name.trim().is_empty() {
        "Sheet1".to_string()
    } else {
        template.sheet_name.clone()
    };
    let header_row = template.header_row.max(1) as u32;

    if !path.exists() {
        return Ok(SheetTableDto {
            path: path.to_string_lossy().to_string(),
            sheet: sheet_name,
            header_row,
            headers: template
                .columns
                .iter()
                .map(|column| column.label.clone())
                .collect(),
            keys: template
                .columns
                .iter()
                .map(|column| column.key.clone())
                .collect(),
            rows: Vec::new(),
        });
    }

    let table = ledger::read_sheet_table(&path, &template.sheet_name, header_row)?;
    Ok(SheetTableDto {
        path: path.to_string_lossy().to_string(),
        sheet: table.name,
        header_row,
        keys: column_keys(&template.columns, table.headers.len()),
        headers: table.headers,
        rows: table
            .rows
            .into_iter()
            .map(|row| SheetRowDto {
                row: row.index,
                cells: row.cells,
            })
            .collect(),
    })
}

/// 列出模板工作簿里的全部工作表；文件还没生成时返回空列表。
#[tauri::command]
fn read_template_sheets(
    template_id: String,
    db: State<DbState>,
) -> Result<Vec<SheetInfoDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let Some(path) = resolve_export_path(&template) else {
        return Ok(Vec::new());
    };
    if !path.exists() {
        return Ok(Vec::new());
    }
    let sheets = ledger::read_workbook(&path)?;
    Ok(sheets
        .into_iter()
        .map(|sheet| SheetInfoDto {
            name: sheet.name,
            headers: sheet.headers,
            rows: sheet.rows,
        })
        .collect())
}

/// 按名称读取模板工作簿里的任意工作表，用于名录页按 sheet 浏览。
#[tauri::command]
fn read_sheet_data_named(
    template_id: String,
    sheet_name: String,
    db: State<DbState>,
) -> Result<SheetTableDto, String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let header_row = template
        .sheets
        .iter()
        .find(|sheet| sheet.name == sheet_name)
        .map(|sheet| sheet.header_row)
        .unwrap_or(template.header_row)
        .max(1) as u32;

    if !path.exists() {
        let columns = sheet_columns(&template, &sheet_name);
        return Ok(SheetTableDto {
            path: path.to_string_lossy().to_string(),
            sheet: sheet_name,
            header_row,
            headers: columns.iter().map(|column| column.label.clone()).collect(),
            keys: columns.iter().map(|column| column.key.clone()).collect(),
            rows: Vec::new(),
        });
    }

    let table = ledger::read_sheet_table(&path, &sheet_name, header_row)?;
    let columns = sheet_columns(&template, &sheet_name);
    Ok(SheetTableDto {
        path: path.to_string_lossy().to_string(),
        sheet: table.name,
        header_row,
        keys: column_keys(&columns, table.headers.len()),
        headers: table.headers,
        rows: table
            .rows
            .into_iter()
            .map(|row| SheetRowDto {
                row: row.index,
                cells: row.cells,
            })
            .collect(),
    })
}

/// 只把改动过的列写回 Excel，未改动的单元格原样保留。
#[tauri::command]
fn update_sheet_row(
    template_id: String,
    sheet_name: Option<String>,
    row: u32,
    values: Vec<Option<String>>,
    db: State<DbState>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let sheet_name = resolve_sheet_name(&template, sheet_name);
    let columns = sheet_columns(&template, &sheet_name);
    let mapped: Vec<Option<ledger::CellValue>> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_deref()
                .map(|text| column_cell_value(&columns, index, text))
        })
        .collect();
    ledger::update_row(&path, &sheet_name, row, &mapped)
}

/// 批量写回多行，区域粘贴 / 撤销重做只落一次盘。
#[tauri::command]
fn update_sheet_rows(
    template_id: String,
    sheet_name: Option<String>,
    rows: Vec<SheetRowUpdate>,
    db: State<DbState>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let sheet_name = resolve_sheet_name(&template, sheet_name);
    let columns = sheet_columns(&template, &sheet_name);
    let updates: Vec<(u32, Vec<Option<ledger::CellValue>>)> = rows
        .iter()
        .map(|update| {
            let mapped = update
                .values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    value
                        .as_deref()
                        .map(|text| column_cell_value(&columns, index, text))
                })
                .collect();
            (update.row, mapped)
        })
        .collect();
    ledger::update_rows(&path, &sheet_name, &updates)
}

/// 批量删除多行，只落一次盘。
#[tauri::command]
fn delete_sheet_rows(
    template_id: String,
    sheet_name: Option<String>,
    rows: Vec<u32>,
    db: State<DbState>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let sheet_name = resolve_sheet_name(&template, sheet_name);
    ledger::delete_rows(&path, &sheet_name, &rows)
}

#[tauri::command]
fn delete_sheet_row(
    template_id: String,
    sheet_name: Option<String>,
    row: u32,
    db: State<DbState>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let sheet_name = resolve_sheet_name(&template, sheet_name);
    ledger::delete_row(&path, &sheet_name, row)
}

#[tauri::command]
fn reorder_sheet_rows(
    template_id: String,
    sheet_name: Option<String>,
    rows: Vec<u32>,
    db: State<DbState>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let sheet_name = resolve_sheet_name(&template, sheet_name);
    ledger::reorder_rows(
        &path,
        &sheet_name,
        sheet_header_row(&template, &sheet_name),
        &rows,
    )
}

#[tauri::command]
fn append_sheet_row(
    template_id: String,
    sheet_name: Option<String>,
    values: Vec<String>,
    db: State<DbState>,
) -> Result<u32, String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("无法创建导出目录：{}", e))?;
        }
    }
    let sheet_name = resolve_sheet_name(&template, sheet_name);
    let columns = sheet_columns(&template, &sheet_name);
    let headers: Vec<String> = columns.iter().map(|column| column.label.clone()).collect();
    if !path.exists() {
        let workbook_sheets: Vec<(String, Vec<String>)> = template
            .sheets
            .iter()
            .map(|sheet| {
                (
                    sheet.name.clone(),
                    sheet
                        .columns
                        .iter()
                        .map(|column| column.label.clone())
                        .collect(),
                )
            })
            .collect();
        if workbook_sheets.is_empty() {
            ledger::create_workbook_with_sheets(&path, &[(sheet_name.clone(), headers)])?;
        } else {
            ledger::create_workbook_with_sheets(&path, &workbook_sheets)?;
        }
    }
    let cells: Vec<ledger::CellValue> = values
        .iter()
        .enumerate()
        .map(|(index, value)| column_cell_value(&columns, index, value))
        .collect();
    ledger::append_row_aligned(
        &path,
        &sheet_name,
        sheet_header_row(&template, &sheet_name),
        &cells,
    )
}

/// 修改表头文字，同时更新导出配置里的列名，保证后续追加还能匹配到这一列。
#[tauri::command]
fn rename_sheet_column(
    template_id: String,
    column: u32,
    label: String,
    db: State<DbState>,
) -> Result<(), String> {
    let label = label.trim().to_string();
    if label.is_empty() {
        return Err("列名不能为空".to_string());
    }
    if column == 0 {
        return Err("列号无效".to_string());
    }
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let index = (column - 1) as usize;
    let header_row = template.header_row.max(1) as u32;

    if !path.exists() {
        let mut columns = template.columns.clone();
        let Some(target) = columns.get_mut(index) else {
            return Err("列不存在，请先完成一次追加导出".to_string());
        };
        target.label = label;
        return save_template_columns(&mut conn, &template, columns);
    }

    let table = ledger::read_sheet_table(&path, &template.sheet_name, header_row)?;
    if index >= table.headers.len() {
        return Err("列不存在，请刷新后重试".to_string());
    }
    if table
        .headers
        .iter()
        .enumerate()
        .any(|(position, header)| position != index && header == &label)
    {
        return Err("列名不能重复".to_string());
    }

    let mut values: Vec<Option<ledger::CellValue>> = vec![None; index + 1];
    values[index] = Some(ledger::CellValue::Text(label.clone()));
    ledger::update_row(&path, &template.sheet_name, header_row, &values)?;

    let mut columns = template.columns.clone();
    pad_template_columns(&mut columns, &table.headers);
    if let Some(target) = columns.get_mut(index) {
        target.label = label;
    }
    save_template_columns(&mut conn, &template, columns)
}

/// 在表格末尾增加一列，并同步到导出配置。
#[tauri::command]
fn add_sheet_column(
    template_id: String,
    label: String,
    db: State<DbState>,
) -> Result<(), String> {
    let label = label.trim().to_string();
    if label.is_empty() {
        return Err("列名不能为空".to_string());
    }
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let header_row = template.header_row.max(1) as u32;

    if !path.exists() {
        let mut columns = template.columns.clone();
        if columns.iter().any(|column| column.label == label) {
            return Err("列名不能重复".to_string());
        }
        let mut headers: Vec<String> = columns
            .iter()
            .map(|column| column.label.clone())
            .collect();
        headers.push(label.clone());
        ledger::sync_headers(&path, &template.sheet_name, 1, &headers)?;
        let key = next_column_key(&columns, columns.len());
        columns.push(TemplateColumn {
            key,
            label,
            width: 18,
            format: "text".to_string(),
            hint: String::new(),
            options: Vec::new(),
        });
        return save_template_columns(&mut conn, &template, columns);
    }

    let table = ledger::read_sheet_table(&path, &template.sheet_name, header_row)?;
    if table.headers.iter().any(|header| header == &label) {
        return Err("列名不能重复".to_string());
    }
    let next_col = table.headers.len() as u32 + 1;
    let mut values: Vec<Option<ledger::CellValue>> = vec![None; next_col as usize];
    values[(next_col - 1) as usize] = Some(ledger::CellValue::Text(label.clone()));
    ledger::update_row(&path, &template.sheet_name, header_row, &values)?;

    let mut columns = template.columns.clone();
    pad_template_columns(&mut columns, &table.headers);
    let key = next_column_key(&columns, table.headers.len());
    columns.insert(
        table.headers.len(),
        TemplateColumn {
            key,
            label,
            width: 18,
            format: "text".to_string(),
            hint: String::new(),
            options: Vec::new(),
        },
    );
    save_template_columns(&mut conn, &template, columns)
}

/// 删除一列（连同这一列的数据），并同步到导出配置。
#[tauri::command]
fn remove_sheet_column(
    template_id: String,
    column: u32,
    db: State<DbState>,
) -> Result<(), String> {
    if column == 0 {
        return Err("列号无效".to_string());
    }
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let index = (column - 1) as usize;
    let header_row = template.header_row.max(1) as u32;

    if !path.exists() {
        let mut columns = template.columns.clone();
        if columns.len() <= 1 {
            return Err("至少保留一列".to_string());
        }
        if index >= columns.len() {
            return Err("列不存在".to_string());
        }
        columns.remove(index);
        return save_template_columns(&mut conn, &template, columns);
    }

    let table = ledger::read_sheet_table(&path, &template.sheet_name, header_row)?;
    if table.headers.len() <= 1 {
        return Err("至少保留一列".to_string());
    }
    if index >= table.headers.len() {
        return Err("列不存在，请刷新后重试".to_string());
    }
    ledger::remove_column(&path, &template.sheet_name, column)?;

    let mut columns = template.columns.clone();
    pad_template_columns(&mut columns, &table.headers);
    if index < columns.len() {
        columns.remove(index);
    }
    save_template_columns(&mut conn, &template, columns)
}

/// 一次性保存列设置：列名、示例、宽度、格式，以及列的增删和顺序。
#[tauri::command]
fn save_sheet_columns(
    template_id: String,
    sheet_name: Option<String>,
    columns: Vec<SheetColumnInput>,
    db: State<DbState>,
) -> Result<(), String> {
    if columns.is_empty() {
        return Err("至少保留一列".to_string());
    }
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let sheet_name = resolve_sheet_name(&template, sheet_name);
    let header_row = sheet_header_row(&template, &sheet_name);
    let existing_columns = sheet_columns(&template, &sheet_name);

    let mut sources = std::collections::HashSet::new();
    for column in &columns {
        let label = column.label.trim();
        if label.is_empty() {
            return Err("列名不能为空".to_string());
        }
        if column.source > 0 && !sources.insert(column.source) {
            return Err("列设置有重复，请刷新后重试".to_string());
        }
    }

    if !path.exists() {
        let headers: Vec<String> = columns
            .iter()
            .map(|column| column.label.trim().to_string())
            .collect();
        let mut workbook_sheets: Vec<(String, Vec<String>)> = template
            .sheets
            .iter()
            .map(|sheet| {
                (
                    sheet.name.clone(),
                    sheet
                        .columns
                        .iter()
                        .map(|column| column.label.clone())
                        .collect(),
                )
            })
            .collect();
        if let Some(target) = workbook_sheets
            .iter_mut()
            .find(|(name, _)| name == &sheet_name)
        {
            target.1 = headers;
        } else {
            workbook_sheets.push((sheet_name.clone(), headers));
        }
        ledger::create_workbook_with_sheets(&path, &workbook_sheets)?;
    } else {
        let specs: Vec<ledger::ColumnSpec> = columns
            .iter()
            .map(|column| ledger::ColumnSpec {
                source: column.source,
                label: column.label.trim().to_string(),
                width: column.width,
            })
            .collect();
        ledger::save_columns(&path, &sheet_name, header_row, &specs)?;
    }

    let mut used: std::collections::HashSet<String> = existing_columns
        .iter()
        .map(|column| column.key.clone())
        .collect();
    let mut next_columns: Vec<TemplateColumn> = Vec::with_capacity(columns.len());
    for (index, column) in columns.iter().enumerate() {
        let key = if column.source > 0 {
            existing_columns
                .get((column.source - 1) as usize)
                .map(|item| item.key.clone())
                .filter(|key| !key.is_empty())
                .unwrap_or_else(|| next_column_key_from(&used, index))
        } else {
            next_column_key_from(&used, index)
        };
        used.insert(key.clone());
        let format = match column.format.as_str() {
            "number" => "number",
            "date" => "date",
            "select" => "select",
            "multi_select" | "multiSelect" => "multi_select",
            _ => "text",
        };
        next_columns.push(TemplateColumn {
            key,
            label: column.label.trim().to_string(),
            width: column.width.clamp(6, 80),
            format: format.to_string(),
            hint: column.hint.trim().to_string(),
            options: column
                .options
                .iter()
                .map(|option| option.trim().to_string())
                .filter(|option| !option.is_empty())
                .collect(),
        });
    }
    save_sheet_columns_for(&mut conn, &template, &sheet_name, next_columns)
}

#[tauri::command]
fn save_template(input: TemplateInput, db: State<DbState>) -> Result<TemplateDto, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let saved = db::save_template(&mut conn, &input).map_err(db_err)?;
    let Some(path) = resolve_export_path(&saved) else {
        return Ok(saved);
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let sheets: Vec<(String, Vec<String>)> = saved
        .sheets
        .iter()
        .map(|sheet| {
            (
                sheet.name.clone(),
                sheet
                    .columns
                    .iter()
                    .map(|column| column.label.clone())
                    .collect(),
            )
        })
        .collect();
    if !sheets.iter().any(|(_, headers)| !headers.is_empty()) {
        return Ok(saved);
    }

    if !path.exists() {
        ledger::create_workbook_with_sheets(&path, &sheets)?;
        return Ok(saved);
    }

    // 工作簿已存在：主表必须同步成功，其余工作表尽力而为。
    for sheet in &saved.sheets {
        if sheet.columns.is_empty() {
            continue;
        }
        let headers: Vec<String> = sheet
            .columns
            .iter()
            .map(|column| column.label.clone())
            .collect();
        let result = ledger::sync_headers(
            &path,
            &sheet.name,
            sheet.header_row.max(1) as u32,
            &headers,
        );
        if sheet.primary {
            result?;
        }
    }
    Ok(saved)
}

/// 在模板工作簿末尾新建一个工作表，并同步到模板定义。
#[tauri::command]
fn add_template_sheet(template_id: String, db: State<DbState>) -> Result<String, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut used: std::collections::HashSet<String> = template
        .sheets
        .iter()
        .map(|sheet| sheet.name.clone())
        .collect();
    if path.exists() {
        for sheet in ledger::read_workbook(&path)? {
            used.insert(sheet.name);
        }
    }
    let mut counter = used.len() + 1;
    let mut name = format!("Sheet{}", counter);
    while used.contains(&name) {
        counter += 1;
        name = format!("Sheet{}", counter);
    }

    let headers = vec!["名称".to_string()];
    if path.exists() {
        ledger::append_worksheet(&path, &name, &headers)?;
    } else {
        let mut workbook_sheets: Vec<(String, Vec<String>)> = template
            .sheets
            .iter()
            .map(|sheet| {
                (
                    sheet.name.clone(),
                    sheet
                        .columns
                        .iter()
                        .map(|column| column.label.clone())
                        .collect(),
                )
            })
            .collect();
        workbook_sheets.push((name.clone(), headers));
        ledger::create_workbook_with_sheets(&path, &workbook_sheets)?;
    }

    let mut sheets = template.sheets.clone();
    sheets.push(TemplateSheet {
        name: name.clone(),
        header_row: 1,
        columns: vec![TemplateColumn {
            key: "col_1".to_string(),
            label: "名称".to_string(),
            width: 24,
            format: "text".to_string(),
            hint: String::new(),
            options: Vec::new(),
        }],
        primary: false,
    });
    let input = TemplateInput {
        id: Some(template.id.clone()),
        name: template.name.clone(),
        columns: template.columns.clone(),
        target_path: template.target_path.clone(),
        sheet_name: template.sheet_name.clone(),
        header_row: template.header_row,
        file_name_rule: template.file_name_rule.clone(),
        is_default: template.is_default,
        sheets,
    };
    db::save_template(&mut conn, &input).map_err(db_err)?;
    Ok(name)
}

pub(crate) fn validate_sheet_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("工作表名称不能为空".to_string());
    }
    if name.chars().count() > 31 {
        return Err("工作表名称不能超过 31 个字符".to_string());
    }
    if name.chars().any(|ch| "\\/:*?[]".contains(ch)) {
        return Err("工作表名称不能包含 \\ / : * ? [ ]".to_string());
    }
    if name.starts_with('\'') || name.ends_with('\'') {
        return Err("工作表名称不能以单引号开头或结尾".to_string());
    }
    if name.eq_ignore_ascii_case("history") {
        return Err("History 是保留名称，请换一个".to_string());
    }
    Ok(())
}

/// 重命名工作表，并同步模板定义。
#[tauri::command]
fn rename_template_sheet(
    template_id: String,
    sheet_name: String,
    new_name: String,
    db: State<DbState>,
) -> Result<(), String> {
    let name = new_name.trim().to_string();
    validate_sheet_name(&name)?;
    if name == sheet_name {
        return Ok(());
    }

    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    if !template.sheets.iter().any(|sheet| sheet.name == sheet_name) {
        return Err(format!("模板里没有工作表「{}」", sheet_name));
    }
    if template
        .sheets
        .iter()
        .any(|sheet| sheet.name != sheet_name && sheet.name.eq_ignore_ascii_case(&name))
    {
        return Err(format!("工作表「{}」已存在", name));
    }

    if let Some(path) = resolve_export_path(&template) {
        if path.exists() {
            ledger::rename_sheet(&path, &sheet_name, &name)?;
        }
    }

    let mut sheets = template.sheets.clone();
    if let Some(target) = sheets.iter_mut().find(|sheet| sheet.name == sheet_name) {
        target.name = name.clone();
    }
    let primary = sheets
        .iter()
        .find(|sheet| sheet.primary)
        .or_else(|| sheets.first())
        .cloned()
        .unwrap_or(TemplateSheet {
            name: template.sheet_name.clone(),
            header_row: template.header_row,
            columns: template.columns.clone(),
            primary: true,
        });
    let input = TemplateInput {
        id: Some(template.id.clone()),
        name: template.name.clone(),
        columns: primary.columns.clone(),
        target_path: template.target_path.clone(),
        sheet_name: primary.name.clone(),
        header_row: primary.header_row,
        file_name_rule: template.file_name_rule.clone(),
        is_default: template.is_default,
        sheets,
    };
    db::save_template(&mut conn, &input).map_err(db_err)?;
    Ok(())
}

#[tauri::command]
fn delete_template(id: String, db: State<DbState>) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    db::delete_template(&mut conn, &id)
}

#[tauri::command]
fn set_default_template(id: String, db: State<DbState>) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::set_default_template(&conn, &id).map_err(db_err)
}

#[tauri::command]
fn list_basket(db: State<DbState>) -> Result<Vec<MessageDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::list_basket(&conn).map_err(db_err)
}

#[tauri::command]
fn add_to_basket(ids: Vec<String>, db: State<DbState>) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::add_to_basket(&conn, &ids).map_err(db_err)
}

#[tauri::command]
fn remove_from_basket(ids: Vec<String>, db: State<DbState>) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::remove_from_basket(&conn, &ids).map_err(db_err)
}

#[tauri::command]
fn clear_basket(db: State<DbState>) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::clear_basket(&conn).map_err(db_err)
}

/// 追加到现有台账：按表头列名匹配、缺列补列、成功后打批次标记。
#[tauri::command]
fn append_ledger(
    ids: Vec<String>,
    template_id: String,
    path: String,
    include_exported: bool,
    db: State<DbState>,
) -> Result<AppendReportDto, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = db::get_template(&conn, &template_id)
        .map_err(db_err)?
        .or_else(|| db::list_templates(&conn).ok()?.into_iter().next())
        .ok_or("没有可用的导出配置")?;
    let target_path = if path.trim().is_empty() {
        resolve_export_path(&template).ok_or("请先在导出设置里选择目标目录")?
    } else {
        let picked = PathBuf::from(path.trim());
        if path.trim().to_lowercase().ends_with(".xlsx") {
            picked
        } else {
            picked.join(export_file_name(&template))
        }
    };
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("无法创建导出目录：{}", e))?;
    }
    let target = target_path.to_string_lossy().to_string();

    let all = db::list_records(&conn).map_err(db_err)?;
    let wanted: Vec<RecordDto> = all
        .into_iter()
        .filter(|record| ids.is_empty() || ids.iter().any(|id| id == &record.id))
        .collect();
    let mut skipped = 0i64;
    let mut rows: Vec<Vec<ledger::CellValue>> = Vec::new();
    let mut plain_values: Vec<String> = Vec::new();
    let mut written_ids: Vec<String> = Vec::new();
    let headerless = template.columns.is_empty();
    for record in &wanted {
        if !include_exported && record.exported_at.is_some() {
            skipped += 1;
            continue;
        }
        if headerless {
            let text = record
                .data
                .get("excerpt")
                .and_then(|value| value.as_str())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| record.title.clone());
            plain_values.push(text);
        } else {
            let row: Vec<ledger::CellValue> = template
                .columns
                .iter()
                .map(|column| {
                    let value = record
                        .data
                        .get(&column.key)
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .to_string();
                    if column.format == "number" {
                        match value.trim().parse::<f64>() {
                            Ok(number) => ledger::CellValue::Number(number),
                            Err(_) => ledger::CellValue::Text(value),
                        }
                    } else {
                        ledger::CellValue::Text(value)
                    }
                })
                .collect();
            rows.push(row);
        }
        written_ids.push(record.id.clone());
    }
    if written_ids.is_empty() {
        return Err(if skipped > 0 {
            "选中的记录都已导出".to_string()
        } else {
            "没有可追加的记录".to_string()
        });
    }

    let report = if headerless {
        ledger::append_plain_column(&target_path, &template.sheet_name, &plain_values)?
    } else {
        let headers: Vec<String> = template
            .columns
            .iter()
            .map(|column| column.label.clone())
            .collect();
        ledger::append_to_workbook(
            &target_path,
            &template.sheet_name,
            template.header_row.max(1) as u32,
            &headers,
            &rows,
        )?
    };
    let batch = format!("B{}", chrono::Local::now().format("%Y%m%d%H%M%S"));
    db::mark_exported(&mut conn, &written_ids, &batch, &target).map_err(db_err)?;

    Ok(AppendReportDto {
        written: report.written as i64,
        skipped,
        added_columns: report.added_columns,
        path: target,
        batch,
        sheet: report.sheet,
    })
}

/// 导入台账时优先用显式路径，其次用模板里配置的台账文件。
fn import_workbook_path(template: &TemplateDto, path: Option<String>) -> Result<PathBuf, String> {
    if let Some(raw) = path
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        let picked = PathBuf::from(raw);
        if picked.is_dir() {
            return Err("请选择 .xlsx 台账文件".to_string());
        }
        return Ok(picked);
    }
    resolve_export_path(template).ok_or_else(|| "请先在导出设置里选择台账文件".to_string())
}

fn ledger_import_preview_dto(
    plan: &ledger_import::ImportPlan,
    template_id: &str,
) -> LedgerImportPreviewDto {
    let (total_excel_rows, total_matched, total_excel_only, total_record_only, total_updated) =
        plan.totals();
    LedgerImportPreviewDto {
        template_id: template_id.to_string(),
        workbook: plan.workbook.clone(),
        sheets: plan
            .sheets
            .iter()
            .map(|sheet| LedgerImportSheetDto {
                name: sheet.counts.name.clone(),
                excel_rows: sheet.counts.excel_rows,
                matched: sheet.counts.matched,
                excel_only: sheet.counts.excel_only,
                record_only: sheet.counts.record_only,
                updated: sheet.counts.updated,
            })
            .collect(),
        total_excel_rows,
        total_matched,
        total_excel_only,
        total_record_only,
        total_updated,
        columns_added: plan.columns_added(),
        ignored_sheets: plan.ignored_sheets.clone(),
    }
}

fn ledger_import_backup_path(data_dir: &std::path::Path) -> PathBuf {
    let dir = data_dir.join("backups");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("jy-collector.pre-ledger-import.db")
}

/// 只读对账：返回每个工作表的 Excel 行数、可匹配行数、仅在 Excel / 仅在记录的行数。
#[tauri::command]
fn ledger_import_preview(
    template_id: String,
    path: Option<String>,
    db: State<DbState>,
) -> Result<LedgerImportPreviewDto, String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let workbook = import_workbook_path(&template, path)?;
    let plan = ledger_import::build_plan(&conn, &template, &workbook)?;
    Ok(ledger_import_preview_dto(&plan, &template.id))
}

/// 落库导入：先备份数据库，再按 policy 覆盖或保留记录值。
#[tauri::command]
fn ledger_import_apply(
    template_id: String,
    path: Option<String>,
    policy: Option<String>,
    db: State<DbState>,
    data_dir: State<DataDirState>,
) -> Result<LedgerImportReportDto, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let workbook = import_workbook_path(&template, path)?;
    let backup = ledger_import_backup_path(&data_dir.0);
    if !backup.exists() {
        conn.execute(
            "VACUUM INTO ?1",
            rusqlite::params![backup.to_string_lossy().to_string()],
        )
        .map_err(|error| format!("备份数据库失败：{}", error))?;
    }
    let policy = match policy.as_deref() {
        Some("record") => "record",
        _ => "excel",
    };
    let (plan, outcome) = ledger_import::apply(&mut conn, &template, &workbook, policy)?;
    Ok(LedgerImportReportDto {
        preview: ledger_import_preview_dto(&plan, &template.id),
        inserted: outcome.inserted,
        updated: outcome.updated,
        policy: policy.to_string(),
        backup_path: backup.to_string_lossy().to_string(),
    })
}

/// 回滚上一次导入：把 records / record_sources / templates 还原到导入前。
/// 还原前会再备份一次当前数据库，误操作也能退回来。
#[tauri::command]
fn ledger_import_restore(
    db: State<DbState>,
    data_dir: State<DataDirState>,
) -> Result<i64, String> {
    let backup = ledger_import_backup_path(&data_dir.0);
    if !backup.exists() {
        return Err("没有可回滚的导入备份".to_string());
    }
    let mut conn = db.0.lock().map_err(db_err)?;
    let stamp = chrono::Local::now().format("%Y%m%d%H%M%S");
    let safety = data_dir
        .0
        .join("backups")
        .join(format!("jy-collector.before-restore-{}.db", stamp));
    conn.execute(
        "VACUUM INTO ?1",
        rusqlite::params![safety.to_string_lossy().to_string()],
    )
    .map_err(|error| format!("备份当前数据失败：{}", error))?;

    conn.execute(
        "ATTACH DATABASE ?1 AS bak",
        rusqlite::params![backup.to_string_lossy().to_string()],
    )
    .map_err(|error| format!("无法打开备份：{}", error))?;

    let outcome = (|| -> Result<i64, String> {
        let live_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        let backup_version: i64 = conn
            .query_row("PRAGMA bak.user_version", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        if live_version != backup_version {
            return Err("备份与当前数据结构版本不一致，不能自动回滚".to_string());
        }
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute_batch(
            "DELETE FROM record_sources;
             DELETE FROM records;
             DELETE FROM templates;
             INSERT INTO templates SELECT * FROM bak.templates;
             INSERT INTO records SELECT * FROM bak.records;
             INSERT INTO record_sources SELECT * FROM bak.record_sources;",
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        conn.query_row("SELECT COUNT(*) FROM records", [], |row| row.get(0))
            .map_err(|e| e.to_string())
    })();

    let _ = conn.execute("DETACH DATABASE bak", []);
    outcome
}

/// 读取数据库台账；返回稳定行 id，前端后续写入都用 id。
#[tauri::command]
fn read_ledger_rows(
    template_id: String,
    sheet_name: Option<String>,
    db: State<DbState>,
) -> Result<LedgerTableDto, String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    ledger_store::read_rows(&conn, &template, sheet_name.as_deref())
}

/// 工作表列表与行数，全部来自数据库，不再打开 Excel。
#[tauri::command]
fn read_ledger_sheets(
    template_id: String,
    db: State<DbState>,
) -> Result<Vec<SheetInfoDto>, String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    ledger_store::list_sheets(&conn, &template)
}

#[tauri::command]
fn update_ledger_rows(
    template_id: String,
    sheet_name: Option<String>,
    rows: Vec<LedgerRowUpdate>,
    db: State<DbState>,
) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let sheet = ledger_store::resolve_sheet(&template, sheet_name.as_deref())?;
    ledger_store::update_rows(&mut conn, &template, &sheet, &rows)
}

#[tauri::command]
fn delete_ledger_rows(
    template_id: String,
    sheet_name: Option<String>,
    ids: Vec<String>,
    db: State<DbState>,
) -> Result<usize, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let sheet = ledger_store::resolve_sheet(&template, sheet_name.as_deref())?;
    ledger_store::delete_rows(&mut conn, &template, &sheet, &ids)
}

#[tauri::command]
fn append_ledger_rows(
    template_id: String,
    sheet_name: Option<String>,
    rows: Vec<Vec<String>>,
    db: State<DbState>,
) -> Result<Vec<String>, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let sheet = ledger_store::resolve_sheet(&template, sheet_name.as_deref())?;
    ledger_store::append_rows(&mut conn, &template, &sheet, &rows)
}

#[tauri::command]
fn reorder_ledger_rows(
    template_id: String,
    sheet_name: Option<String>,
    ids: Vec<String>,
    db: State<DbState>,
) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let sheet = ledger_store::resolve_sheet(&template, sheet_name.as_deref())?;
    ledger_store::reorder_rows(&mut conn, &template, &sheet, &ids)
}

#[tauri::command]
fn save_ledger_columns(
    template_id: String,
    sheet_name: Option<String>,
    columns: Vec<SheetColumnInput>,
    db: State<DbState>,
) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let sheet = ledger_store::resolve_sheet(&template, sheet_name.as_deref())?;
    ledger_store::save_columns(&mut conn, &template, &sheet, &columns)
}

#[tauri::command]
fn add_ledger_sheet(
    template_id: String,
    name: Option<String>,
    db: State<DbState>,
) -> Result<String, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    ledger_store::add_sheet(&mut conn, &template, name)
}

#[tauri::command]
fn rename_ledger_sheet(
    template_id: String,
    old_name: String,
    new_name: String,
    db: State<DbState>,
) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    ledger_store::rename_sheet(&mut conn, &template, &old_name, &new_name)
}

#[tauri::command]
fn delete_ledger_sheet(
    template_id: String,
    name: String,
    force: Option<bool>,
    db: State<DbState>,
) -> Result<usize, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    ledger_store::delete_sheet(&mut conn, &template, &name, force.unwrap_or(false))
}

/// 放到工作线程执行，避免全量解密期间界面卡住、进度不动。
#[tauri::command(async)]
fn refresh_source(
    db: State<DbState>,
    source: State<wechat::SourceState>,
) -> Result<SyncSummaryDto, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let result = wechat::sync(&mut conn, &source.sidecar_dir);
    drop(conn);
    record_sync_result(&source, &result);
    result
}

#[tauri::command]
fn source_status(source: State<wechat::SourceState>) -> Result<SourceStatusDto, String> {
    let status = source.status.lock().map_err(db_err)?;
    Ok(status.clone())
}

/// 首次启动向导 / 重新授权：检测微信目录、口令与口令有效性。
/// 每次都重新读钥匙串，所以“已有口令”会直接返回 ready。
#[tauri::command]
async fn wechat_setup_status(
    source: State<'_, wechat::SourceState>,
) -> Result<SetupStatusDto, String> {
    let dir = source.sidecar_dir.clone();
    tauri::async_runtime::spawn_blocking(move || wechat::setup_status(&dir))
        .await
        .map_err(|_| "检测失败，请重试".to_string())
}

const SETUP_COMPLETED_KEY: &str = "setup_completed";

#[tauri::command]
fn setup_completed(db: State<DbState>) -> Result<bool, String> {
    let conn = db.0.lock().map_err(db_err)?;
    Ok(db::setting_value(&conn, SETUP_COMPLETED_KEY)
        .map_err(db_err)?
        .as_deref()
        == Some("1"))
}

/// 向导走完后写一次性标志；之后启动直接进主界面。
#[tauri::command]
fn complete_setup(db: State<DbState>) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    db::save_setting(&conn, SETUP_COMPLETED_KEY, "1").map_err(db_err)
}

/// 数据有更新未刷新：文件未变动时只做 stat；
/// 变动静默 20 秒且距上次解密 ≥30 秒时，在后台线程只解一次 session.db，
/// 对比关注群的最后消息时间与本地游标后决定是否提示。
/// 微信未运行或读库失败时静默返回 false。
#[tauri::command]
async fn source_updated(
    db: State<'_, DbState>,
    source: State<'_, wechat::SourceState>,
) -> Result<bool, String> {
    if wechat::is_pending(&source) {
        return Ok(true);
    }
    if !wechat::claim_check(&source) {
        return Ok(false);
    }

    // 只短暂持锁读取关注范围与游标，解密期间不占用数据库。
    let scope = match db.0.lock() {
        Ok(conn) => match db::capture_scope(&conn) {
            Ok((groups, _, configured)) => Some((
                groups,
                configured,
                db::wechat_cursors(&conn).unwrap_or_default(),
            )),
            Err(_) => None,
        },
        Err(_) => None,
    };
    let Some((watched, configured, cursors)) = scope else {
        wechat::release_check(&source);
        return Ok(false);
    };

    // 没有勾选任何群：退回全局提示，不再解密。
    if !configured || watched.is_empty() {
        wechat::release_check(&source);
        return Ok(wechat::set_pending(&source, true));
    }

    let generation = wechat::current_generation(&source);
    let sidecar_dir = source.sidecar_dir.clone();
    let account = wechat::account(&source);
    let probe_watched = watched.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        wechat::session_last_timestamps(&sidecar_dir, &account, &probe_watched)
    })
    .await;
    wechat::release_check(&source);

    match result {
        Ok(Ok(stamps)) => {
            // 期间完成过一次真正的同步：以同步结果为准。
            if wechat::current_generation(&source) != generation {
                return Ok(wechat::is_pending(&source));
            }
            let changed = watched.iter().any(|group| {
                stamps.get(group).copied().unwrap_or(0)
                    > cursors.get(group).copied().unwrap_or(0)
            });
            Ok(wechat::set_pending(&source, changed))
        }
        _ => Ok(false),
    }
}

#[tauri::command]
fn get_data_dir(data_dir: State<DataDirState>) -> String {
    data_dir.0.to_string_lossy().to_string()
}

#[tauri::command]
fn open_data_dir(data_dir: State<DataDirState>) -> Result<(), String> {
    open_path(&data_dir.0).map_err(db_err)
}

fn open_path(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");

    command.arg(path).spawn()?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let mut conn = db::open(&data_dir.join("jy-collector.db"))?;

            // 启动时先同步一次，让界面直接拿到真实群/消息；
            // 失败不影响窗口打开，错误会通过 source_status 交给前端提示。
            let source = wechat::SourceState::new(resolve_sidecar_dir(app.handle()));
            let result = wechat::sync(&mut conn, &source.sidecar_dir);
            record_sync_result(&source, &result);

            app.manage(DbState(Mutex::new(conn)));
            app.manage(DataDirState(data_dir));
            app.manage(source);
            if let Some(window) = app.get_webview_window("main") {
                // macOS 用自绘顶栏，清除原生标题；Windows 保留窗口标题，避免任务栏显示空白。
                #[cfg(target_os = "macos")]
                let _ = window.set_title("");
                #[cfg(not(target_os = "macos"))]
                let _ = window.set_title("JY Collector");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            list_groups,
            list_messages,
            list_person_messages,
            list_avatars,
            list_avatar_urls,
            sync_avatars,
            load_avatar,
            load_message_image,
            load_message_emoticon,
            search_people,
            list_followed,
            toggle_follow,
            get_message,
            search_messages,
            message_context,
            extract_rows,
            list_records,
            create_record,
            create_records,
            update_record,
            delete_record,
            record_sources,
            save_schemas,
            get_settings,
            save_setting,
            get_capture_scope,
            save_capture_scope,
            export_records,
            list_templates,
            list_workbooks,
            import_workbook,
            read_workbook,
            read_template_sheets,
            read_sheet_data,
            read_sheet_data_named,
            update_sheet_row,
            update_sheet_rows,
            delete_sheet_row,
            delete_sheet_rows,
            reorder_sheet_rows,
            append_sheet_row,
            rename_sheet_column,
            add_sheet_column,
            remove_sheet_column,
            save_sheet_columns,
            save_template,
            add_template_sheet,
            rename_template_sheet,
            delete_template,
            set_default_template,
            list_basket,
            add_to_basket,
            remove_from_basket,
            clear_basket,
            append_ledger,
            ledger_import_preview,
            ledger_import_apply,
            ledger_import_restore,
            read_ledger_rows,
            read_ledger_sheets,
            update_ledger_rows,
            delete_ledger_rows,
            append_ledger_rows,
            reorder_ledger_rows,
            save_ledger_columns,
            add_ledger_sheet,
            rename_ledger_sheet,
            delete_ledger_sheet,
            get_data_dir,
            open_data_dir,
            refresh_source,
            source_status,
            source_updated,
            wechat_setup_status,
            setup_completed,
            complete_setup
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
