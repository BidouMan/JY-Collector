use crate::models::{
    AvatarDto, GroupDto, MessageDto, PersonDto, RecordDto, RecordPayload, RecordSourceDto,
    SchemaDto, SchemaInput, SearchItemDto, SearchPageDto, SearchQuery, SettingsDto, TemplateColumn,
    TemplateDto, TemplateInput, WxAvatarInput, WxGroupInput, WxMessageInput,
};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;

const SCHEMA_SQL: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS chat_groups (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    member_count INTEGER NOT NULL DEFAULT 0,
    last_active TEXT
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES chat_groups(id),
    sender_id TEXT NOT NULL,
    sender_name TEXT NOT NULL,
    message_type TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    quoted_message_id TEXT,
    quoted_content TEXT,
    quoted_sender TEXT,
    file_name TEXT,
    file_size TEXT,
    created_at TEXT NOT NULL,
    source_ts INTEGER
);

CREATE INDEX IF NOT EXISTS idx_messages_group ON messages(group_id);
CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender_id);

CREATE TABLE IF NOT EXISTS records (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES chat_groups(id),
    title TEXT NOT NULL,
    data_json TEXT NOT NULL DEFAULT '{}',
    template_id TEXT,
    exported_at TEXT,
    export_batch TEXT,
    export_target TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS templates (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    columns_json TEXT NOT NULL DEFAULT '[]',
    target_path TEXT NOT NULL DEFAULT '',
    sheet_name TEXT NOT NULL DEFAULT '',
    header_row INTEGER NOT NULL DEFAULT 1,
    file_name_rule TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS basket (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    added_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS record_sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    record_id TEXT NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES messages(id),
    field_key TEXT NOT NULL,
    snippet TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_record_sources_record ON record_sources(record_id);

CREATE TABLE IF NOT EXISTS field_schemas (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES chat_groups(id),
    field_key TEXT NOT NULL,
    label TEXT NOT NULL,
    field_type TEXT NOT NULL,
    required INTEGER NOT NULL DEFAULT 0,
    example TEXT NOT NULL DEFAULT '',
    sort_order INTEGER NOT NULL DEFAULT 0,
    UNIQUE(group_id, field_key)
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS capture_scope (
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    PRIMARY KEY (entity_type, entity_id)
);

CREATE TABLE IF NOT EXISTS avatars (
    entity_id TEXT PRIMARY KEY,
    mime TEXT NOT NULL,
    data_b64 TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

/// 新发现的群默认带上一套通用字段，保证「选中消息 -> 录入」开箱可用。
/// 字段与规则抽取器（src/services/extractor.ts）中的 key 对应。
const DEFAULT_SCHEMAS: &[(&str, &str, &str, bool, &str)] = &[
    ("student", "学员姓名", "text", true, "张小雨"),
    ("subject", "科目", "text", false, "数学"),
    ("lesson_time", "上课时间", "text", false, "周日 10:00"),
    ("teacher", "授课老师", "text", false, "王老师"),
    ("note", "备注", "textarea", false, "调课需提前一天"),
];

/// 内置导出配置。默认无列定义，首次使用时在下载目录生成空工作簿。
pub const DEFAULT_TEMPLATE_ID: &str = "tpl_green_channel";
const DEFAULT_TEMPLATE_NAME: &str = "默认导出";
const DEFAULT_SHEET_NAME: &str = "sheet1";
const DEFAULT_FILE_NAME: &str = "默认导出.xlsx";

fn default_export_dir() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    let home = std::env::var_os("USERPROFILE").map(std::path::PathBuf::from);
    #[cfg(not(target_os = "windows"))]
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let base = home
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
    base.join("Downloads")
}

fn seed_default_template(conn: &Connection) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM templates", [], |row| row.get(0))?;
    if count > 0 {
        return Ok(());
    }
    let dir = default_export_dir();
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join(DEFAULT_FILE_NAME);
    if !file.exists() {
        let _ = crate::ledger::create_empty_workbook(&file, DEFAULT_SHEET_NAME);
    }
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    conn.execute(
        "INSERT INTO templates
         (id, name, is_default, columns_json, target_path, sheet_name, header_row, file_name_rule, created_at, updated_at)
         VALUES (?1, ?2, 1, '[]', ?3, ?4, 1, ?5, ?6, ?6)",
        params![
            DEFAULT_TEMPLATE_ID,
            DEFAULT_TEMPLATE_NAME,
            dir.to_string_lossy().to_string(),
            DEFAULT_SHEET_NAME,
            DEFAULT_FILE_NAME,
            now
        ],
    )?;
    Ok(())
}

fn template_from_row(row: &rusqlite::Row) -> rusqlite::Result<TemplateDto> {
    let raw: String = row.get(3)?;
    let columns: Vec<TemplateColumn> = serde_json::from_str(&raw).unwrap_or_default();
    Ok(TemplateDto {
        id: row.get(0)?,
        name: row.get(1)?,
        is_default: row.get::<_, i64>(2)? != 0,
        columns,
        target_path: row.get(4)?,
        sheet_name: row.get(5)?,
        header_row: row.get(6)?,
        file_name_rule: row.get(7)?,
    })
}

pub fn list_templates(conn: &Connection) -> rusqlite::Result<Vec<TemplateDto>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, is_default, columns_json, target_path, sheet_name, header_row, file_name_rule
         FROM templates ORDER BY is_default DESC, created_at ASC",
    )?;
    let rows = stmt.query_map([], template_from_row)?;
    rows.collect()
}

pub fn save_template(conn: &mut Connection, input: &TemplateInput) -> rusqlite::Result<TemplateDto> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let id = match input.id.as_deref() {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => next_id("tpl"),
    };
    let name = if input.name.trim().is_empty() {
        "未命名模板".to_string()
    } else {
        input.name.trim().to_string()
    };
    let columns = serde_json::to_string(&input.columns).unwrap_or_else(|_| "[]".into());
    let sheet = if input.sheet_name.trim().is_empty() {
        name.clone()
    } else {
        input.sheet_name.trim().to_string()
    };
    let header_row = if input.header_row < 1 { 1 } else { input.header_row };
    let rule = if input.file_name_rule.trim().is_empty() {
        format!("{}-{{date}}.xlsx", name)
    } else {
        input.file_name_rule.trim().to_string()
    };

    let tx = conn.transaction()?;
    let exists: i64 = tx.query_row(
        "SELECT COUNT(*) FROM templates WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )?;
    if exists > 0 {
        tx.execute(
            "UPDATE templates SET name=?1, columns_json=?2, target_path=?3, sheet_name=?4,
                    header_row=?5, file_name_rule=?6, updated_at=?7 WHERE id=?8",
            params![
                name,
                columns,
                input.target_path.trim(),
                sheet,
                header_row,
                rule,
                now,
                id
            ],
        )?;
    } else {
        tx.execute(
            "INSERT INTO templates
             (id, name, is_default, columns_json, target_path, sheet_name, header_row, file_name_rule, created_at, updated_at)
             VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                id,
                name,
                columns,
                input.target_path.trim(),
                sheet,
                header_row,
                rule,
                now
            ],
        )?;
    }
    if input.is_default {
        tx.execute("UPDATE templates SET is_default = 0", [])?;
        tx.execute("UPDATE templates SET is_default = 1 WHERE id = ?1", params![id])?;
    }
    let has_default: i64 =
        tx.query_row("SELECT COUNT(*) FROM templates WHERE is_default = 1", [], |row| row.get(0))?;
    if has_default == 0 {
        tx.execute("UPDATE templates SET is_default = 1 WHERE id = ?1", params![id])?;
    }
    tx.commit()?;
    get_template(conn, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub fn get_template(conn: &Connection, id: &str) -> rusqlite::Result<Option<TemplateDto>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, is_default, columns_json, target_path, sheet_name, header_row, file_name_rule
         FROM templates WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], template_from_row)?;
    match rows.next() {
        Some(row) => Ok(Some(row?)),
        None => Ok(None),
    }
}

pub fn delete_template(conn: &mut Connection, id: &str) -> Result<(), String> {
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM templates", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if total <= 1 {
        return Err("至少保留一个模板".to_string());
    }
    let used: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM records WHERE template_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if used > 0 {
        return Err(format!("该模板已被 {} 条记录使用，不能删除", used));
    }

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let was_default: i64 = tx
        .query_row(
            "SELECT is_default FROM templates WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(0);
    tx.execute("DELETE FROM templates WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    if was_default != 0 {
        if let Some(next) = tx
            .query_row(
                "SELECT id FROM templates ORDER BY created_at ASC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
        {
            tx.execute("UPDATE templates SET is_default = 1 WHERE id = ?1", params![next])
                .map_err(|e| e.to_string())?;
        }
    }
    let has_default: i64 = tx
        .query_row("SELECT COUNT(*) FROM templates WHERE is_default = 1", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if has_default == 0 {
        if let Some(next) = tx
            .query_row(
                "SELECT id FROM templates ORDER BY created_at ASC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
        {
            tx.execute("UPDATE templates SET is_default = 1 WHERE id = ?1", params![next])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_default_template(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE templates SET is_default = 0", [])?;
    conn.execute("UPDATE templates SET is_default = 1 WHERE id = ?1", params![id])?;
    Ok(())
}

fn next_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}_{}", prefix, millis, seq)
}

pub fn list_basket(conn: &Connection) -> rusqlite::Result<Vec<MessageDto>> {
    let sql = format!(
        "{} JOIN basket b ON b.message_id = m.id ORDER BY b.added_at DESC, m.created_at DESC",
        MESSAGE_SELECT
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| message_from_row(row, 0))?;
    rows.collect()
}

pub fn add_to_basket(conn: &Connection, ids: &[String]) -> rusqlite::Result<()> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    for id in ids {
        conn.execute(
            "INSERT OR IGNORE INTO basket (message_id, added_at) VALUES (?1, ?2)",
            params![id, now],
        )?;
    }
    Ok(())
}

pub fn remove_from_basket(conn: &Connection, ids: &[String]) -> rusqlite::Result<()> {
    for id in ids {
        conn.execute("DELETE FROM basket WHERE message_id = ?1", params![id])?;
    }
    Ok(())
}

pub fn clear_basket(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM basket", [])?;
    Ok(())
}

/// 追加成功后给记录打上批次标记，避免重复追加。
pub fn mark_exported(
    conn: &mut Connection,
    ids: &[String],
    batch: &str,
    target: &str,
) -> rusqlite::Result<()> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let tx = conn.transaction()?;
    for id in ids {
        tx.execute(
            "UPDATE records SET exported_at=?1, export_batch=?2, export_target=?3, updated_at=?1
             WHERE id=?4",
            params![now, batch, target, id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

const MESSAGE_SELECT: &str = r#"
SELECT m.id, m.group_id, g.name, m.sender_id, m.sender_name, m.message_type, m.content,
       m.quoted_message_id, COALESCE(q.content, m.quoted_content),
       COALESCE(q.sender_name, m.quoted_sender), m.file_name, m.file_size, m.created_at
FROM messages m
JOIN chat_groups g ON g.id = m.group_id
LEFT JOIN messages q ON q.id = m.quoted_message_id
"#;

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.execute_batch(SCHEMA_SQL)?;
    migrate(&conn)?;
    cleanup_legacy_mock(&mut conn)?;
    seed_default_template(&conn)?;
    Ok(conn)
}

/// 旧版本数据库补列（SQLite 的 ADD COLUMN 是 O(1)，不会重写整库）。
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    ensure_column(conn, "chat_groups", "last_active", "TEXT")?;
    ensure_column(conn, "messages", "quoted_content", "TEXT")?;
    ensure_column(conn, "messages", "quoted_sender", "TEXT")?;
    ensure_column(conn, "messages", "source_ts", "INTEGER")?;
    ensure_column(conn, "records", "template_id", "TEXT")?;
    ensure_column(conn, "records", "exported_at", "TEXT")?;
    ensure_column(conn, "records", "export_batch", "TEXT")?;
    ensure_column(conn, "records", "export_target", "TEXT")?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, ddl: &str) -> rusqlite::Result<()> {
    let exists = conn
        .prepare(&format!("PRAGMA table_info({})", table))?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .any(|name| name == column);
    if !exists {
        conn.execute_batch(&format!(
            "ALTER TABLE {} ADD COLUMN {} {}",
            table, column, ddl
        ))?;
    }
    Ok(())
}

/// 早期版本用 seed.json 写过 55 条模拟消息（群 id 固定为 g1..g4）。
/// 接入真实数据后一次性清掉，避免和真实群混在一起。
fn cleanup_legacy_mock(conn: &mut Connection) -> rusqlite::Result<()> {
    let done: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'mock_cleanup_done'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if done.as_deref() == Some("1") {
        return Ok(());
    }

    let tx = conn.transaction()?;
    tx.execute_batch(
        r#"
        DELETE FROM record_sources
         WHERE record_id IN (SELECT id FROM records WHERE group_id IN ('g1','g2','g3','g4'))
            OR message_id IN (SELECT id FROM messages WHERE group_id IN ('g1','g2','g3','g4'));
        DELETE FROM records WHERE group_id IN ('g1','g2','g3','g4');
        DELETE FROM field_schemas WHERE group_id IN ('g1','g2','g3','g4');
        DELETE FROM messages WHERE group_id IN ('g1','g2','g3','g4');
        DELETE FROM chat_groups WHERE id IN ('g1','g2','g3','g4');
        INSERT INTO settings (key, value) VALUES ('mock_cleanup_done', '1')
          ON CONFLICT(key) DO UPDATE SET value = excluded.value;
        "#,
    )?;
    tx.commit()?;
    Ok(())
}

fn message_from_row(row: &rusqlite::Row, offset: usize) -> rusqlite::Result<MessageDto> {
    Ok(MessageDto {
        id: row.get(offset)?,
        group_id: row.get(offset + 1)?,
        group_name: row.get(offset + 2)?,
        sender_id: row.get(offset + 3)?,
        sender_name: row.get(offset + 4)?,
        message_type: row.get(offset + 5)?,
        content: row.get(offset + 6)?,
        quoted_message_id: row.get(offset + 7)?,
        quoted_content: row.get(offset + 8)?,
        quoted_sender: row.get(offset + 9)?,
        file_name: row.get(offset + 10)?,
        file_size: row.get(offset + 11)?,
        created_at: row.get(offset + 12)?,
    })
}

/// 采集范围开关：保存过之后，采集页/查找页只展示勾选的群组与联系人。
const SCOPE_CONFIGURED_KEY: &str = "capture_scope_configured";

/// 消息级范围条件（m 为 messages 表别名）。群被勾选或发送人被勾选即视为在范围内。
const SCOPE_MESSAGE_FILTER: &str = "(m.group_id IN (SELECT entity_id FROM capture_scope WHERE entity_type = 'group') \
     OR m.sender_id IN (SELECT entity_id FROM capture_scope WHERE entity_type = 'person'))";

fn scope_configured(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(setting_value(conn, SCOPE_CONFIGURED_KEY)?.as_deref() == Some("1"))
}

/// 读取已保存的采集范围（群 id、联系人 id、是否已配置）。
pub fn capture_scope(conn: &Connection) -> rusqlite::Result<(Vec<String>, Vec<String>, bool)> {
    let configured = scope_configured(conn)?;
    if !configured {
        return Ok((Vec::new(), Vec::new(), false));
    }
    let mut stmt = conn.prepare("SELECT entity_type, entity_id FROM capture_scope")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut groups = Vec::new();
    let mut people = Vec::new();
    for row in rows {
        let (kind, id) = row?;
        match kind.as_str() {
            "group" => groups.push(id),
            "person" => people.push(id),
            _ => {}
        }
    }
    Ok((groups, people, true))
}

/// 保存采集范围：整体覆盖写入，并打开范围开关。
pub fn save_capture_scope(
    conn: &mut Connection,
    groups: &[String],
    people: &[String],
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM capture_scope", [])?;
    for id in groups.iter().filter(|id| !id.trim().is_empty()) {
        tx.execute(
            "INSERT OR IGNORE INTO capture_scope (entity_type, entity_id) VALUES ('group', ?1)",
            params![id],
        )?;
    }
    for id in people.iter().filter(|id| !id.trim().is_empty()) {
        tx.execute(
            "INSERT OR IGNORE INTO capture_scope (entity_type, entity_id) VALUES ('person', ?1)",
            params![id],
        )?;
    }
    tx.execute(
        "INSERT INTO settings (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = '1'",
        params![SCOPE_CONFIGURED_KEY],
    )?;
    tx.commit()
}

pub fn list_groups(conn: &Connection) -> rusqlite::Result<Vec<GroupDto>> {
    list_groups_where(conn, "")
}

/// 采集/查找页使用的群列表：配置过采集范围后只返回勾选的群。
pub fn list_groups_scoped(conn: &Connection) -> rusqlite::Result<Vec<GroupDto>> {
    if !scope_configured(conn)? {
        return list_groups(conn);
    }
    list_groups_where(
        conn,
        "WHERE g.id IN (SELECT entity_id FROM capture_scope WHERE entity_type = 'group')",
    )
}

fn list_groups_where(conn: &Connection, filter: &str) -> rusqlite::Result<Vec<GroupDto>> {
    let sql = format!(
        r#"
        SELECT g.id, g.name, g.member_count,
               (SELECT COUNT(*) FROM messages m WHERE m.group_id = g.id),
               (SELECT COUNT(*) FROM records r WHERE r.group_id = g.id),
               COALESCE(NULLIF(g.last_active, ''),
                        (SELECT MAX(m.created_at) FROM messages m WHERE m.group_id = g.id))
        FROM chat_groups g
        {}
        ORDER BY 6 DESC, g.name ASC
    "#,
        filter
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(GroupDto {
            id: row.get(0)?,
            name: row.get(1)?,
            member_count: row.get(2)?,
            message_count: row.get(3)?,
            record_count: row.get(4)?,
            last_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

fn epoch_to_local(seconds: i64) -> Option<String> {    if seconds <= 0 {
        return None;
    }
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|utc| utc.with_timezone(&chrono::Local))
        .map(|local| local.format("%Y-%m-%d %H:%M:%S").to_string())
}

pub fn setting_value(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0))
        .optional()
}

/// 每个群已导入到的最新消息时间（sidecar 的增量游标）。
pub fn wechat_cursors(conn: &Connection) -> rusqlite::Result<HashMap<String, i64>> {    let mut stmt = conn.prepare(
        "SELECT group_id, MAX(source_ts) FROM messages WHERE source_ts IS NOT NULL GROUP BY group_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (group_id, ts) = row?;
        out.insert(group_id, ts);
    }
    Ok(out)
}

/// 把 sidecar 解出的群/消息/头像写入应用库。
/// 返回 (总消息数, 本次新增消息数)。
pub fn apply_wechat_sync(
    conn: &mut Connection,
    groups: &[WxGroupInput],
    messages: &[WxMessageInput],
    avatars: &[WxAvatarInput],
) -> rusqlite::Result<(i64, i64)> {
    let before: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;
    let tx = conn.transaction()?;

    for group in groups {
        tx.execute(
            "INSERT INTO chat_groups (id, name, member_count, last_active)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                member_count = excluded.member_count,
                last_active = excluded.last_active",
            params![
                group.id,
                group.name,
                group.member_count,
                epoch_to_local(group.last_active.unwrap_or(0))
            ],
        )?;
        for (index, (key, label, field_type, required, example)) in
            DEFAULT_SCHEMAS.iter().enumerate()
        {
            let id = format!("sch_{}_{}", group.id, key);
            tx.execute(
                "INSERT OR IGNORE INTO field_schemas
                 (id, group_id, field_key, label, field_type, required, example, sort_order)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id,
                    group.id,
                    key,
                    label,
                    field_type,
                    if *required { 1 } else { 0 },
                    example,
                    index as i64
                ],
            )?;
        }
    }

    for message in messages {
        let id = format!("wx:{}:{}", message.group_id, message.local_id);
        tx.execute(
            "INSERT INTO messages
                (id, group_id, sender_id, sender_name, message_type, content,
                 quoted_message_id, quoted_content, quoted_sender,
                 file_name, file_size, created_at, source_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET
                sender_id = excluded.sender_id,
                sender_name = excluded.sender_name,
                message_type = excluded.message_type,
                content = excluded.content,
                quoted_content = excluded.quoted_content,
                quoted_sender = excluded.quoted_sender,
                file_name = excluded.file_name,
                file_size = excluded.file_size,
                created_at = excluded.created_at,
                source_ts = excluded.source_ts",
            params![
                id,
                message.group_id,
                message.sender_id,
                message.sender_name,
                message.message_type,
                message.content.clone().unwrap_or_default(),
                message.quoted_content,
                message.quoted_sender,
                message.file_name,
                message.file_size,
                epoch_to_local(message.created_at).unwrap_or_else(|| "1970-01-01 00:00:00".into()),
                message.created_at
            ],
        )?;
    }

    upsert_avatars(&tx, avatars)?;

    tx.commit()?;
    let after: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;
    Ok((after, after - before))
}

fn upsert_avatars(conn: &Connection, avatars: &[WxAvatarInput]) -> rusqlite::Result<()> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    for avatar in avatars {
        if avatar.entity_id.is_empty() || avatar.data.is_empty() {
            continue;
        }
        conn.execute(
            "INSERT INTO avatars (entity_id, mime, data_b64, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(entity_id) DO UPDATE SET
                mime = excluded.mime,
                data_b64 = excluded.data_b64,
                updated_at = excluded.updated_at",
            params![avatar.entity_id, avatar.mime, avatar.data, now],
        )?;
    }
    Ok(())
}

/// 单独写入头像（不触发消息同步）。
pub fn apply_avatars(conn: &mut Connection, avatars: &[WxAvatarInput]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    upsert_avatars(&tx, avatars)?;
    tx.commit()
}

/// 读取全部头像（entityId 为 wxid 或群 username）。
pub fn list_avatars(conn: &Connection) -> rusqlite::Result<Vec<AvatarDto>> {
    let mut stmt = conn.prepare("SELECT entity_id, mime, data_b64 FROM avatars")?;
    let rows = stmt.query_map([], |row| {
        Ok(AvatarDto {
            entity_id: row.get(0)?,
            mime: row.get(1)?,
            data: row.get(2)?,
        })
    })?;
    rows.collect()
}

pub fn list_messages(conn: &Connection, group_id: &str) -> rusqlite::Result<Vec<MessageDto>> {
    let sql = format!(
        "{} WHERE m.group_id = ?1 ORDER BY m.created_at ASC, m.id ASC",
        MESSAGE_SELECT
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![group_id], |row| message_from_row(row, 0))?;
    rows.collect()
}

pub fn list_person_messages(conn: &Connection, sender_id: &str) -> rusqlite::Result<Vec<MessageDto>> {
    let mut sql = format!(
        "{} WHERE m.sender_id = ?1",
        MESSAGE_SELECT
    );
    if scope_configured(conn)? {
        sql.push_str(&format!(" AND {}", SCOPE_MESSAGE_FILTER));
    }
    sql.push_str(" ORDER BY m.created_at ASC, m.id ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![sender_id], |row| message_from_row(row, 0))?;
    rows.collect()
}

pub fn get_message(conn: &Connection, message_id: &str) -> rusqlite::Result<Option<MessageDto>> {
    let sql = format!("{} WHERE m.id = ?1", MESSAGE_SELECT);
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query_map(params![message_id], |row| message_from_row(row, 0))?;
    match rows.next() {
        Some(item) => Ok(Some(item?)),
        None => Ok(None),
    }
}

pub fn search_people(
    conn: &Connection,
    keyword: &str,
    scoped: bool,
) -> rusqlite::Result<Vec<PersonDto>> {
    // 联系人列表只由「联系人」勾选项决定，与群组勾选相互独立。
    let scope_filter = if scoped && scope_configured(conn)? {
        " AND m.sender_id IN (SELECT entity_id FROM capture_scope WHERE entity_type = 'person')"
            .to_string()
    } else {
        String::new()
    };
    let sql = format!(
        r#"
        SELECT m.sender_id, m.sender_name, COUNT(*) AS message_count,
               COUNT(DISTINCT m.group_id) AS group_count,
               GROUP_CONCAT(DISTINCT g.name) AS group_names,
               MAX(m.created_at) AS last_at
        FROM messages m
        JOIN chat_groups g ON g.id = m.group_id
        WHERE (m.sender_name LIKE ?1 OR m.sender_id LIKE ?1){}
        GROUP BY m.sender_id, m.sender_name
        ORDER BY message_count DESC, m.sender_name ASC
    "#,
        scope_filter
    );
    let pattern = format!("%{}%", keyword);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![pattern], |row| {
        let group_names: String = row.get(4)?;
        Ok(PersonDto {
            sender_id: row.get(0)?,
            sender_name: row.get(1)?,
            message_count: row.get(2)?,
            group_count: row.get(3)?,
            group_names: group_names.replace(',', "、"),
            last_at: row.get(5)?,
        })
    })?;
    rows.collect()
}

fn like_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn like_contains(raw: &str) -> String {
    format!("%{}%", like_escape(raw))
}

/// 组装检索的 WHERE 子句与绑定值，供列表与总数共用。
fn search_where(query: &SearchQuery, scope_filter: Option<&str>) -> (String, Vec<Value>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    if let Some(filter) = scope_filter {
        clauses.push(filter.to_string());
    }

    let keyword = query.keyword.trim();
    if !keyword.is_empty() {
        if query.keyword_mode == "exact" {
            clauses.push(
                "(m.content = ? COLLATE NOCASE OR COALESCE(m.quoted_content, '') = ? COLLATE NOCASE)"
                    .into(),
            );
            values.push(Value::Text(keyword.to_string()));
            values.push(Value::Text(keyword.to_string()));
        } else {
            clauses.push(
                "(m.content LIKE ? ESCAPE '\\' OR COALESCE(m.quoted_content, '') LIKE ? ESCAPE '\\' OR COALESCE(m.file_name, '') LIKE ? ESCAPE '\\')"
                    .into(),
            );
            for _ in 0..3 {
                values.push(Value::Text(like_contains(keyword)));
            }
        }
    }

    let group_ids: Vec<&String> = query
        .group_ids
        .iter()
        .filter(|id| !id.trim().is_empty())
        .collect();
    if !group_ids.is_empty() {
        let marks = vec!["?"; group_ids.len()].join(",");
        clauses.push(format!("m.group_id IN ({})", marks));
        for id in group_ids {
            values.push(Value::Text(id.clone()));
        }
    }

    let sender_id = query.sender_id.trim();
    if !sender_id.is_empty() {
        clauses.push("m.sender_id = ?".into());
        values.push(Value::Text(sender_id.to_string()));
    }
    let sender_name = query.sender_name.trim();
    if !sender_name.is_empty() {
        clauses.push("m.sender_name LIKE ? ESCAPE '\\'".into());
        values.push(Value::Text(like_contains(sender_name)));
    }

    let mention = query.mention.trim().trim_start_matches('@').trim();
    if !mention.is_empty() {
        clauses.push("m.content LIKE ? ESCAPE '\\'".into());
        values.push(Value::Text(format!("%@{}%", like_escape(mention))));
    }

    let types: Vec<&String> = query
        .types
        .iter()
        .filter(|item| !item.trim().is_empty())
        .collect();
    if !types.is_empty() {
        let marks = vec!["?"; types.len()].join(",");
        clauses.push(format!("m.message_type IN ({})", marks));
        for item in types {
            values.push(Value::Text(item.clone()));
        }
    }

    let start = query.start_date.trim();
    if !start.is_empty() {
        clauses.push("m.created_at >= ?".into());
        values.push(Value::Text(format!("{} 00:00:00", start)));
    }
    let end = query.end_date.trim();
    if !end.is_empty() {
        clauses.push("m.created_at <= ?".into());
        values.push(Value::Text(format!("{} 23:59:59", end)));
    }

    if let Some(flag) = query.in_basket {
        clauses.push(
            if flag {
                "EXISTS (SELECT 1 FROM basket b WHERE b.message_id = m.id)"
            } else {
                "NOT EXISTS (SELECT 1 FROM basket b WHERE b.message_id = m.id)"
            }
            .into(),
        );
    }
    if let Some(flag) = query.in_record {
        clauses.push(
            if flag {
                "EXISTS (SELECT 1 FROM record_sources s WHERE s.message_id = m.id)"
            } else {
                "NOT EXISTS (SELECT 1 FROM record_sources s WHERE s.message_id = m.id)"
            }
            .into(),
        );
    }

    let sql = if clauses.is_empty() {
        "1=1".to_string()
    } else {
        clauses.join(" AND ")
    };
    (sql, values)
}

pub fn search_messages(
    conn: &Connection,
    query: &SearchQuery,
) -> rusqlite::Result<SearchPageDto> {
    let scope_filter = if scope_configured(conn)? {
        Some(SCOPE_MESSAGE_FILTER)
    } else {
        None
    };
    let (where_sql, mut values) = search_where(query, scope_filter);

    let count_sql = format!("SELECT COUNT(*) FROM messages m WHERE {}", where_sql);
    let total: i64 =
        conn.query_row(&count_sql, params_from_iter(values.iter()), |row| row.get(0))?;

    let order = if query.sort == "asc" { "ASC" } else { "DESC" };
    let limit = if query.limit <= 0 { 50 } else { query.limit.min(200) };
    let offset = query.offset.max(0);
    let sql = format!(
        "SELECT m.id, m.group_id, g.name, m.sender_id, m.sender_name, m.message_type, m.content,
                m.quoted_message_id, COALESCE(q.content, m.quoted_content),
                COALESCE(q.sender_name, m.quoted_sender), m.file_name, m.file_size, m.created_at,
                EXISTS (SELECT 1 FROM basket b WHERE b.message_id = m.id),
                EXISTS (SELECT 1 FROM record_sources s WHERE s.message_id = m.id)
         FROM messages m
         JOIN chat_groups g ON g.id = m.group_id
         LEFT JOIN messages q ON q.id = m.quoted_message_id
         WHERE {}
         ORDER BY m.created_at {}, m.id {}
         LIMIT ? OFFSET ?",
        where_sql, order, order
    );
    values.push(Value::Integer(limit));
    values.push(Value::Integer(offset));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values.iter()), |row| {
        Ok(SearchItemDto {
            message: message_from_row(row, 0)?,
            in_basket: row.get::<_, i64>(13)? != 0,
            in_record: row.get::<_, i64>(14)? != 0,
        })
    })?;
    let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(SearchPageDto { items, total })
}

/// 取一条消息前后相邻的上下文（同群按时间排序）。
pub fn message_context(
    conn: &Connection,
    message_id: &str,
    radius: i64,
) -> rusqlite::Result<Vec<MessageDto>> {
    let target = match get_message(conn, message_id)? {
        Some(value) => value,
        None => return Ok(Vec::new()),
    };
    let span = radius.clamp(1, 30);
    let before_sql = format!(
        "{} WHERE m.group_id = ?1 AND (m.created_at < ?2 OR (m.created_at = ?2 AND m.id < ?3))
         ORDER BY m.created_at DESC, m.id DESC LIMIT ?4",
        MESSAGE_SELECT
    );
    let mut stmt = conn.prepare(&before_sql)?;
    let rows = stmt.query_map(
        params![target.group_id, target.created_at, target.id, span],
        |row| message_from_row(row, 0),
    )?;
    let mut before = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    before.reverse();

    let after_sql = format!(
        "{} WHERE m.group_id = ?1 AND (m.created_at > ?2 OR (m.created_at = ?2 AND m.id > ?3))
         ORDER BY m.created_at ASC, m.id ASC LIMIT ?4",
        MESSAGE_SELECT
    );
    let mut stmt = conn.prepare(&after_sql)?;
    let rows = stmt.query_map(
        params![target.group_id, target.created_at, target.id, span],
        |row| message_from_row(row, 0),
    )?;
    let after = rows.collect::<rusqlite::Result<Vec<_>>>()?;

    let mut result = before;
    result.push(target);
    result.extend(after);
    Ok(result)
}

pub fn list_all_schemas(conn: &Connection) -> rusqlite::Result<Vec<SchemaDto>> {
    let sql = r#"
        SELECT id, group_id, field_key, label, field_type, required, example, sort_order
        FROM field_schemas
        ORDER BY group_id ASC, sort_order ASC
    "#;
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(SchemaDto {
            id: row.get(0)?,
            group_id: row.get(1)?,
            key: row.get(2)?,
            label: row.get(3)?,
            field_type: row.get(4)?,
            required: row.get::<_, i64>(5)? != 0,
            example: row.get(6)?,
            sort_order: row.get(7)?,
        })
    })?;
    rows.collect()
}

pub fn list_records(conn: &Connection) -> rusqlite::Result<Vec<RecordDto>> {
    let sql = r#"
        SELECT r.id, r.group_id, g.name, r.title, r.data_json,
               (SELECT COUNT(*) FROM record_sources s WHERE s.record_id = r.id),
               r.template_id, r.exported_at, r.export_batch, r.export_target,
               r.created_at, r.updated_at
        FROM records r
        JOIN chat_groups g ON g.id = r.group_id
        ORDER BY r.updated_at DESC, r.id DESC
    "#;
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        let raw: String = row.get(4)?;
        let data: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));
        Ok(RecordDto {
            id: row.get(0)?,
            group_id: row.get(1)?,
            group_name: row.get(2)?,
            title: row.get(3)?,
            data,
            source_count: row.get(5)?,
            template_id: row.get(6)?,
            exported_at: row.get(7)?,
            export_batch: row.get(8)?,
            export_target: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    })?;
    rows.collect()
}

pub fn create_record(conn: &mut Connection, payload: &RecordPayload) -> rusqlite::Result<String> {
    let id = next_id("rec");
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let title = if payload.title.trim().is_empty() {
        "未命名记录".to_string()
    } else {
        payload.title.trim().to_string()
    };
    let data_json = serde_json::to_string(&payload.data).unwrap_or_else(|_| "{}".to_string());

    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO records
         (id, group_id, title, data_json, template_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            payload.group_id,
            title,
            data_json,
            payload.template_id,
            now,
            now
        ],
    )?;
    for source in &payload.sources {
        tx.execute(
            "INSERT INTO record_sources (record_id, message_id, field_key, snippet)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, source.message_id, source.field_key, source.snippet],
        )?;
    }
    tx.commit()?;
    Ok(id)
}

pub fn update_record(
    conn: &Connection,
    id: &str,
    title: &str,
    data: &serde_json::Value,
) -> rusqlite::Result<()> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let data_json = serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string());
    let title = if title.trim().is_empty() {
        "未命名记录"
    } else {
        title.trim()
    };
    conn.execute(
        "UPDATE records SET title = ?1, data_json = ?2, updated_at = ?3 WHERE id = ?4",
        params![title, data_json, now, id],
    )?;
    Ok(())
}

pub fn delete_record(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM records WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn list_record_sources(
    conn: &Connection,
    record_id: &str,
) -> rusqlite::Result<Vec<RecordSourceDto>> {
    let mut stmt = conn.prepare(
        "SELECT field_key, snippet, message_id FROM record_sources
         WHERE record_id = ?1 ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(params![record_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        let (field_key, snippet, message_id) = row?;
        if let Some(message) = get_message(conn, &message_id)? {
            result.push(RecordSourceDto {
                field_key,
                snippet,
                message,
            });
        }
    }
    Ok(result)
}

pub fn record_source_texts(conn: &Connection, record_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT m.sender_name, m.created_at, s.snippet
         FROM record_sources s
         JOIN messages m ON m.id = s.message_id
         WHERE s.record_id = ?1
         ORDER BY s.id ASC",
    )?;
    let rows = stmt.query_map(params![record_id], |row| {
        Ok(format!(
            "{} {}：{}",
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?
        ))
    })?;
    rows.collect()
}

pub fn save_schemas(
    conn: &mut Connection,
    group_id: &str,
    fields: &[SchemaInput],
) -> rusqlite::Result<Vec<SchemaDto>> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM field_schemas WHERE group_id = ?1",
        params![group_id],
    )?;
    for (index, field) in fields.iter().enumerate() {
        let key = if field.key.trim().is_empty() {
            format!("field_{}", index)
        } else {
            field.key.trim().to_string()
        };
        let id = format!("sch_{}_{}", group_id, key);
        tx.execute(
            "INSERT INTO field_schemas
             (id, group_id, field_key, label, field_type, required, example, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                group_id,
                key,
                field.label,
                field.field_type,
                if field.required { 1 } else { 0 },
                field.example,
                index as i64
            ],
        )?;
    }
    tx.commit()?;
    let all = list_all_schemas(conn)?;
    Ok(all.into_iter().filter(|s| s.group_id == group_id).collect())
}

fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
}

pub fn get_settings(conn: &Connection, data_dir: &str) -> rusqlite::Result<SettingsDto> {
    Ok(SettingsDto {
        api_key: get_setting(conn, "api_key")?.unwrap_or_default(),
        model_name: get_setting(conn, "model_name")?
            .unwrap_or_else(|| "deepseek-flash".to_string()),
        api_base: get_setting(conn, "api_base")?
            .unwrap_or_else(|| "https://api.deepseek.com/v1".to_string()),
        data_dir: data_dir.to_string(),
    })
}

pub fn save_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use crate::models::RecordSourceInput;

    fn temp_conn() -> (Connection, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "jy-flow-{}-{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let conn = open(&path).unwrap();
        (conn, path)
    }

    fn msg(group: &str, local_id: i64, ts: i64, sender: &str, text: &str) -> WxMessageInput {
        WxMessageInput {
            group_id: group.to_string(),
            local_id,
            sender_id: sender.to_string(),
            sender_name: sender.to_string(),
            message_type: "text".to_string(),
            content: Some(text.to_string()),
            quoted_sender: None,
            quoted_content: None,
            file_name: None,
            file_size: None,
            created_at: ts,
        }
    }

    #[test]
    fn record_flow_and_export() {
        let (mut conn, path) = temp_conn();
        let groups = vec![WxGroupInput {
            id: "123@chatroom".to_string(),
            name: "测试群".to_string(),
            member_count: 8,
            last_active: Some(1_700_000_100),
        }];
        let messages = vec![
            msg("123@chatroom", 1, 1_700_000_000, "张三", "周六下午两点上课，地址是浦江花园3号楼"),
            msg("123@chatroom", 2, 1_700_000_100, "李四", "收到"),
        ];
        let (total, added) = apply_wechat_sync(&mut conn, &groups, &messages, &[]).unwrap();
        assert_eq!(total, 2);
        assert_eq!(added, 2);

        let listed = list_groups(&conn).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "测试群");
        assert_eq!(listed[0].last_at.as_deref(), Some("2023-11-15 06:15:00"));
        assert_eq!(list_all_schemas(&conn).unwrap().len(), DEFAULT_SCHEMAS.len());

        // 重复同步不产生重复消息
        let (total, added) = apply_wechat_sync(&mut conn, &groups, &messages, &[]).unwrap();
        assert_eq!(total, 2);
        assert_eq!(added, 0);

        let messages = list_messages(&conn, "123@chatroom").unwrap();
        assert_eq!(messages.len(), 2);
        let message = messages
            .iter()
            .find(|m| m.content.contains("地址"))
            .unwrap();

        let payload = RecordPayload {
            group_id: "123@chatroom".to_string(),
            title: "周子涵 · 数学".to_string(),
            data: serde_json::json!({ "student": "周子涵", "subject": "数学" }),
            sources: vec![RecordSourceInput {
                message_id: message.id.clone(),
                field_key: "student".to_string(),
                snippet: message.content.chars().take(20).collect(),
            }],
            template_id: Some(DEFAULT_TEMPLATE_ID.to_string()),
        };
        let record_id = create_record(&mut conn, &payload).unwrap();

        let records = list_records(&conn).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].source_count, 1);
        assert_eq!(records[0].data["student"], "周子涵");

        let sources = list_record_sources(&conn, &record_id).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].message.id, message.id);

        let value = serde_json::json!({ "student": "周子涵（改）" });
        update_record(&conn, &record_id, "周子涵（改）", &value).unwrap();
        let records = list_records(&conn).unwrap();
        assert_eq!(records[0].title, "周子涵（改）");
        assert_eq!(records[0].data["student"], "周子涵（改）");

        let group_records = list_groups(&conn)
            .unwrap()
            .into_iter()
            .find(|g| g.id == "123@chatroom")
            .unwrap();
        assert_eq!(group_records.record_count, 1);

        let xlsx = std::env::temp_dir().join("jy-flow-export.xlsx");
        let count = crate::export::export_records(
            &conn,
            &[record_id.clone()],
            DEFAULT_TEMPLATE_ID,
            xlsx.to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(count, 1);
        assert!(std::fs::metadata(&xlsx).unwrap().len() > 1000);
        let _ = std::fs::remove_file(&xlsx);

        delete_record(&conn, &record_id).unwrap();
        assert_eq!(list_records(&conn).unwrap().len(), 0);
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM record_sources", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);

        let people = search_people(&conn, "张", false).unwrap();
        assert_eq!(people.len(), 1);
        let person_messages = list_person_messages(&conn, &people[0].sender_id).unwrap();
        assert_eq!(person_messages.len(), 1);

        save_schemas(&mut conn, "123@chatroom", &[]).unwrap();
        assert!(list_all_schemas(&conn)
            .unwrap()
            .iter()
            .filter(|s| s.group_id == "123@chatroom")
            .count()
            == 0);

        save_setting(&conn, "api_key", "sk-test").unwrap();
        assert_eq!(get_settings(&conn, "/tmp").unwrap().api_key, "sk-test");

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn search_filters_and_context() {
        let (mut conn, path) = temp_conn();
        let groups = vec![
            WxGroupInput {
                id: "g-a@chatroom".to_string(),
                name: "班级群A".to_string(),
                member_count: 5,
                last_active: Some(1_700_000_200),
            },
            WxGroupInput {
                id: "g-b@chatroom".to_string(),
                name: "班级群B".to_string(),
                member_count: 5,
                last_active: Some(1_700_100_000),
            },
        ];
        let messages = vec![
            msg("g-a@chatroom", 1, 1_700_000_000, "张三", "@李四 周六上课"),
            msg("g-a@chatroom", 2, 1_700_000_100, "李四", "收到"),
            msg("g-a@chatroom", 3, 1_700_000_200, "张三", "收到"),
            msg("g-b@chatroom", 4, 1_700_100_000, "王五", "周六请假"),
        ];
        apply_wechat_sync(&mut conn, &groups, &messages, &[]).unwrap();

        let fuzzy = SearchQuery {
            keyword: "周六".to_string(),
            keyword_mode: "fuzzy".to_string(),
            ..Default::default()
        };
        let page = search_messages(&conn, &fuzzy).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items.len(), 2);

        let exact = SearchQuery {
            keyword: "收到".to_string(),
            keyword_mode: "exact".to_string(),
            ..Default::default()
        };
        assert_eq!(search_messages(&conn, &exact).unwrap().total, 2);

        let scoped = SearchQuery {
            group_ids: vec!["g-a@chatroom".to_string()],
            sender_name: "张三".to_string(),
            ..Default::default()
        };
        assert_eq!(search_messages(&conn, &scoped).unwrap().total, 2);

        let mention = SearchQuery {
            mention: "@李四".to_string(),
            ..Default::default()
        };
        assert_eq!(search_messages(&conn, &mention).unwrap().total, 1);

        let day = list_messages(&conn, "g-a@chatroom").unwrap()[0].created_at[..10].to_string();
        let ranged = SearchQuery {
            start_date: day.clone(),
            end_date: day,
            ..Default::default()
        };
        assert_eq!(search_messages(&conn, &ranged).unwrap().total, 3);

        let target = list_messages(&conn, "g-b@chatroom").unwrap()[0].id.clone();
        add_to_basket(&conn, &[target.clone()]).unwrap();
        let in_basket = SearchQuery {
            in_basket: Some(true),
            ..Default::default()
        };
        let page = search_messages(&conn, &in_basket).unwrap();
        assert_eq!(page.total, 1);
        assert!(page.items[0].in_basket);
        assert!(!page.items[0].in_record);

        let context = message_context(&conn, &target, 1).unwrap();
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].id, target);

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn capture_scope_limits_visible_content() {
        let (mut conn, path) = temp_conn();
        let groups = vec![
            WxGroupInput {
                id: "g-a@chatroom".to_string(),
                name: "班级群A".to_string(),
                member_count: 5,
                last_active: Some(1_700_000_200),
            },
            WxGroupInput {
                id: "g-b@chatroom".to_string(),
                name: "班级群B".to_string(),
                member_count: 5,
                last_active: Some(1_700_100_000),
            },
        ];
        let messages = vec![
            msg("g-a@chatroom", 1, 1_700_000_000, "张三", "群里消息"),
            msg("g-a@chatroom", 2, 1_700_000_100, "李四", "群里消息二"),
            msg("g-b@chatroom", 3, 1_700_000_200, "王五", "另一个群"),
            msg("g-b@chatroom", 4, 1_700_000_300, "张三", "另一个群里的张三"),
        ];
        apply_wechat_sync(&mut conn, &groups, &messages, &[]).unwrap();

        // 未配置时全部可见
        assert_eq!(list_groups_scoped(&conn).unwrap().len(), 2);
        assert_eq!(search_people(&conn, "", true).unwrap().len(), 3);
        assert_eq!(search_messages(&conn, &Default::default()).unwrap().total, 4);

        // 只保留群 A 和联系人王五
        save_capture_scope(
            &mut conn,
            &["g-a@chatroom".to_string()],
            &["王五".to_string()],
        )
        .unwrap();

        let scoped_groups = list_groups_scoped(&conn).unwrap();
        assert_eq!(scoped_groups.len(), 1);
        assert_eq!(scoped_groups[0].id, "g-a@chatroom");

        // 联系人列表只展示明确勾选的联系人，与群组勾选无关
        let names: Vec<String> = search_people(&conn, "", true)
            .unwrap()
            .into_iter()
            .map(|person| person.sender_name)
            .collect();
        assert_eq!(names, vec!["王五"]);

        // 消息：群 A 的 2 条 + 王五的 1 条（张三在群 B 的发言不可见）
        assert_eq!(search_messages(&conn, &Default::default()).unwrap().total, 3);
        // 未勾选的联系人仍按群组范围看到群 A 内的发言
        let zhang_san = search_people(&conn, "张三", false).unwrap();
        assert_eq!(
            list_person_messages(&conn, &zhang_san[0].sender_id)
                .unwrap()
                .len(),
            1
        );

        let (groups, people, configured) = capture_scope(&conn).unwrap();
        assert!(configured);
        assert_eq!(groups, vec!["g-a@chatroom"]);
        assert_eq!(people, vec!["王五"]);

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn avatars_upsert_and_list() {
        let (mut conn, path) = temp_conn();
        let groups = vec![WxGroupInput {
            id: "123@chatroom".to_string(),
            name: "测试群".to_string(),
            member_count: 1,
            last_active: Some(1_700_000_000),
        }];
        let avatar = WxAvatarInput {
            entity_id: "wxid_test".to_string(),
            mime: "image/jpeg".to_string(),
            data: "AAAA".to_string(),
        };
        apply_wechat_sync(&mut conn, &groups, &[], &[avatar]).unwrap();
        let listed = list_avatars(&conn).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].entity_id, "wxid_test");

        // 同一实体再次同步时应更新而不是重复插入
        let updated = WxAvatarInput {
            entity_id: "wxid_test".to_string(),
            mime: "image/png".to_string(),
            data: "BBBB".to_string(),
        };
        apply_wechat_sync(&mut conn, &groups, &[], &[updated]).unwrap();
        let listed = list_avatars(&conn).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].mime, "image/png");
        assert_eq!(listed[0].data, "BBBB");

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
}
