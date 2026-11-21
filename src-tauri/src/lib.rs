mod db;
mod export;
mod extract;
mod ledger;
mod models;
mod wechat;

use models::{
    AppendReportDto, AvatarDto, BootstrapDto, CaptureScopeDto, GroupDto, MessageDto, PersonDto,
    RecordDto, RecordPayload, RecordSourceDto, SchemaDto, SchemaInput, SearchPageDto, SearchQuery,
    SettingsDto, SheetInfoDto, SheetRowDto, SheetTableDto, SourceStatusDto, SyncSummaryDto,
    TemplateDto, TemplateInput,
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
                status.last_synced_at = chrono::Local::now()
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string();
            }
            Some(summary.account.clone())
        }
        Err(message) => {
            if let Ok(mut status) = source.status.lock() {
                status.last_error = message.clone();
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
    let avatars = wechat::avatar_snapshot(&source.sidecar_dir, &account)?;
    let count = avatars.len();
    let mut conn = db.0.lock().map_err(db_err)?;
    db::apply_avatars(&mut conn, &avatars).map_err(db_err)?;
    Ok(count)
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

fn column_cell_value(template: &TemplateDto, index: usize, text: &str) -> ledger::CellValue {
    let is_number = template
        .columns
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
            rows: Vec::new(),
        });
    }

    let table = ledger::read_sheet_table(&path, &template.sheet_name, header_row)?;
    Ok(SheetTableDto {
        path: path.to_string_lossy().to_string(),
        sheet: table.name,
        header_row,
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
    row: u32,
    values: Vec<Option<String>>,
    db: State<DbState>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    let mapped: Vec<Option<ledger::CellValue>> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_deref()
                .map(|text| column_cell_value(&template, index, text))
        })
        .collect();
    ledger::update_row(&path, &template.sheet_name, row, &mapped)
}

#[tauri::command]
fn delete_sheet_row(template_id: String, row: u32, db: State<DbState>) -> Result<(), String> {
    let conn = db.0.lock().map_err(db_err)?;
    let template = resolve_template(&conn, &template_id)?;
    let path = resolve_export_path(&template).ok_or("请先在设置里选择目标目录")?;
    ledger::delete_row(&path, &template.sheet_name, row)
}

#[tauri::command]
fn append_sheet_row(
    template_id: String,
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
    let headers: Vec<String> = template
        .columns
        .iter()
        .map(|column| column.label.clone())
        .collect();
    if !path.exists() {
        if headers.is_empty() {
            ledger::create_empty_workbook(&path, &template.sheet_name)?;
        } else {
            ledger::sync_headers(&path, &template.sheet_name, 1, &headers)?;
        }
    }
    let cells: Vec<ledger::CellValue> = values
        .iter()
        .enumerate()
        .map(|(index, value)| column_cell_value(&template, index, value))
        .collect();
    ledger::append_row_aligned(
        &path,
        &template.sheet_name,
        template.header_row.max(1) as u32,
        &cells,
    )
}

#[tauri::command]
fn save_template(input: TemplateInput, db: State<DbState>) -> Result<TemplateDto, String> {
    let mut conn = db.0.lock().map_err(db_err)?;
    let saved = db::save_template(&mut conn, &input).map_err(db_err)?;
    if !saved.columns.is_empty() {
        if let Some(path) = resolve_export_path(&saved) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let headers: Vec<String> = saved
                .columns
                .iter()
                .map(|column| column.label.clone())
                .collect();
            ledger::sync_headers(
                &path,
                &saved.sheet_name,
                saved.header_row.max(1) as u32,
                &headers,
            )?;
        }
    }
    Ok(saved)
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

#[tauri::command]
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
                let _ = window.set_title("群录");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            list_groups,
            list_messages,
            list_person_messages,
            list_avatars,
            sync_avatars,
            search_people,
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
            read_workbook,
            read_sheet_data,
            update_sheet_row,
            delete_sheet_row,
            append_sheet_row,
            save_template,
            delete_template,
            set_default_template,
            list_basket,
            add_to_basket,
            remove_from_basket,
            clear_basket,
            append_ledger,
            get_data_dir,
            open_data_dir,
            refresh_source,
            source_status,
            source_updated
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
