//! 台账行的数据库读写。
//!
//! 数据页以后只和这张表打交道：行有稳定 id，顺序由 sort_order 决定，
//! 每个单元格按模板列 key 存在 data_json 里。Excel 只在导出时生成。

use crate::db;
use crate::models::{
    LedgerRowDto, LedgerRowUpdate, LedgerTableDto, SheetColumnInput, SheetInfoDto,
    TemplateColumn, TemplateDto, TemplateSheet,
};
use crate::{next_column_key_from, sheet_columns, validate_sheet_name};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{Map, Value};
use std::collections::HashSet;

const ROW_QUERY: &str =
    "SELECT id, data_json FROM records
      WHERE template_id = ?1 AND sheet_name = ?2
      ORDER BY sort_order ASC, id ASC";

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn header_row_of(template: &TemplateDto, sheet: &str) -> u32 {
    template
        .sheets
        .iter()
        .find(|item| item.name == sheet)
        .map(|item| item.header_row)
        .unwrap_or(template.header_row)
        .max(1) as u32
}

fn ensure_sheet(template: &TemplateDto, sheet: &str) -> Result<(), String> {
    if template.sheets.iter().any(|item| item.name == sheet) {
        Ok(())
    } else {
        Err(format!("找不到工作表「{}」", sheet))
    }
}

/// 没有配置列时沿用旧的无表头模式：整行内容放在 excerpt 一列里展示。
fn effective_columns(template: &TemplateDto, sheet: &str) -> Vec<TemplateColumn> {
    let columns = sheet_columns(template, sheet);
    if !columns.is_empty() {
        return columns;
    }
    vec![TemplateColumn {
        key: "excerpt".to_string(),
        label: "内容".to_string(),
        width: 60,
        format: "text".to_string(),
        hint: String::new(),
        options: Vec::new(),
    }]
}

/// 请求的工作表不存在时明确报错，避免写到旧名字上。
pub fn resolve_sheet(
    template: &TemplateDto,
    sheet_name: Option<&str>,
) -> Result<String, String> {
    if let Some(name) = sheet_name.map(str::trim).filter(|name| !name.is_empty()) {
        ensure_sheet(template, name)?;
        return Ok(name.to_string());
    }
    Ok(db::primary_sheet_name(template))
}

fn data_from_cells(columns: &[TemplateColumn], cells: &[String]) -> Map<String, Value> {
    let mut data = Map::new();
    for (index, column) in columns.iter().enumerate() {
        let Some(value) = cells.get(index) else {
            break;
        };
        if !value.trim().is_empty() {
            data.insert(column.key.clone(), Value::String(value.clone()));
        }
    }
    data
}

fn title_from(cells: &[String], index: usize) -> String {
    let text = cells
        .iter()
        .map(|cell| cell.trim())
        .find(|cell| !cell.is_empty())
        .unwrap_or("");
    if text.is_empty() {
        format!("新增行 {}", index + 1)
    } else {
        text.chars().take(60).collect()
    }
}

fn max_sort_order(
    conn: &Connection,
    template_id: &str,
    sheet: &str,
) -> rusqlite::Result<f64> {
    conn.query_row(
        "SELECT COALESCE(MAX(sort_order), 0) FROM records
          WHERE template_id = ?1 AND sheet_name = ?2",
        params![template_id, sheet],
        |row| row.get(0),
    )
}

/// 读取整张台账；`row` 只用于界面展示，写入一律用 `id`。
pub fn read_rows(
    conn: &Connection,
    template: &TemplateDto,
    sheet_name: Option<&str>,
) -> Result<LedgerTableDto, String> {
    let sheet = resolve_sheet(template, sheet_name)?;
    let columns = effective_columns(template, &sheet);
    let keys: Vec<String> = columns.iter().map(|column| column.key.clone()).collect();
    let headers: Vec<String> = columns.iter().map(|column| column.label.clone()).collect();

    let mut stmt = conn.prepare(ROW_QUERY).map_err(|e| e.to_string())?;
    let mapped = stmt
        .query_map(params![template.id, sheet], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;

    let mut rows = Vec::new();
    for (index, item) in mapped.enumerate() {
        let (id, raw) = item.map_err(|e| e.to_string())?;
        let data: Map<String, Value> = serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let cells = keys
            .iter()
            .map(|key| data.get(key).map(value_text).unwrap_or_default())
            .collect();
        rows.push(LedgerRowDto {
            id,
            row: index as u32 + 1,
            cells,
        });
    }

    let header_row = header_row_of(template, &sheet);
    Ok(LedgerTableDto {
        template_id: template.id.clone(),
        sheet,
        header_row,
        headers,
        keys,
        rows,
    })
}

/// 批量写单元格：`None` 表示保留原值，空字符串表示清空该格。
pub fn update_rows(
    conn: &mut Connection,
    template: &TemplateDto,
    sheet: &str,
    updates: &[LedgerRowUpdate],
) -> Result<(), String> {
    if updates.is_empty() {
        return Ok(());
    }
    ensure_sheet(template, sheet)?;
    let columns = effective_columns(template, sheet);
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for update in updates {
        let raw: Option<String> = tx
            .query_row(
                "SELECT data_json FROM records
                  WHERE id = ?1 AND template_id = ?2 AND sheet_name = ?3",
                params![update.id, template.id, sheet],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(raw) = raw else {
            return Err("有行已被删除或移动，请刷新后重试".to_string());
        };
        let mut data: Map<String, Value> = serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        for (index, value) in update.values.iter().enumerate() {
            let Some(column) = columns.get(index) else {
                break;
            };
            let Some(value) = value else {
                continue;
            };
            if value.trim().is_empty() {
                data.remove(&column.key);
            } else {
                data.insert(column.key.clone(), Value::String(value.clone()));
            }
        }
        let json = serde_json::to_string(&Value::Object(data)).unwrap_or_else(|_| "{}".to_string());
        tx.execute(
            "UPDATE records SET data_json = ?1, updated_at = ?2 WHERE id = ?3",
            params![json, now, update.id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

/// 按 id 删除，返回实际删除行数；只删属于当前模板和工作表的行。
pub fn delete_rows(
    conn: &mut Connection,
    template: &TemplateDto,
    sheet: &str,
    ids: &[String],
) -> Result<usize, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    ensure_sheet(template, sheet)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut deleted = 0usize;
    {
        let mut stmt = tx
            .prepare(
                "DELETE FROM records
                  WHERE id = ?1 AND template_id = ?2 AND sheet_name = ?3",
            )
            .map_err(|e| e.to_string())?;
        for id in ids {
            deleted += stmt
                .execute(params![id, template.id, sheet])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(deleted)
}

/// 手工新增台账行，来源标记为 manual，排在当前工作表末尾。
pub fn append_rows(
    conn: &mut Connection,
    template: &TemplateDto,
    sheet: &str,
    rows: &[Vec<String>],
) -> Result<Vec<String>, String> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    ensure_sheet(template, sheet)?;
    let columns = effective_columns(template, sheet);
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let base = max_sort_order(&tx, &template.id, sheet).map_err(|e| e.to_string())?;
    let mut ids = Vec::with_capacity(rows.len());
    for (index, cells) in rows.iter().enumerate() {
        let id = db::next_id("rec");
        let data = data_from_cells(&columns, cells);
        let json =
            serde_json::to_string(&Value::Object(data)).unwrap_or_else(|_| "{}".to_string());
        tx.execute(
            "INSERT INTO records
             (id, group_id, title, data_json, template_id, sheet_name, sort_order, origin, created_at, updated_at)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, 'manual', ?7, ?7)",
            params![
                id,
                title_from(cells, index),
                json,
                template.id,
                sheet,
                base + index as f64 + 1.0,
                now
            ],
        )
        .map_err(|e| e.to_string())?;
        ids.push(id);
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(ids)
}

/// 按给定顺序重排：下标即新的行序。
pub fn reorder_rows(
    conn: &mut Connection,
    template: &TemplateDto,
    sheet: &str,
    ids: &[String],
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    ensure_sheet(template, sheet)?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    {
        let mut stmt = tx
            .prepare(
                "UPDATE records SET sort_order = ?1, updated_at = ?2
                  WHERE id = ?3 AND template_id = ?4 AND sheet_name = ?5",
            )
            .map_err(|e| e.to_string())?;
        for (index, id) in ids.iter().enumerate() {
            stmt.execute(params![
                index as f64 + 1.0,
                now,
                id,
                template.id,
                sheet
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

/// 删除列时要同步清掉所有行里对应的 key，避免留下看不见的数据。
fn scrub_keys(
    conn: &Connection,
    template_id: &str,
    sheet: &str,
    keys: &[String],
    now: &str,
) -> rusqlite::Result<()> {
    let rows: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, data_json FROM records
              WHERE template_id = ?1 AND sheet_name = ?2",
        )?;
        let mapped = stmt.query_map(params![template_id, sheet], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        mapped.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut update = conn.prepare("UPDATE records SET data_json = ?1, updated_at = ?2 WHERE id = ?3")?;
    for (id, raw) in rows {
        let Ok(Value::Object(mut data)) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let before = data.len();
        for key in keys {
            data.remove(key);
        }
        if data.len() != before {
            update.execute(params![Value::Object(data).to_string(), now, id])?;
        }
    }
    Ok(())
}

/// 保存列设置：改名沿用原 key，新增列分配新 key，被删列同步清空数据。
pub fn save_columns(
    conn: &mut Connection,
    template: &TemplateDto,
    sheet: &str,
    inputs: &[SheetColumnInput],
) -> Result<(), String> {
    ensure_sheet(template, sheet)?;
    if inputs.is_empty() {
        return Err("至少保留一列".to_string());
    }
    let current = effective_columns(template, sheet);
    let mut used: HashSet<String> = HashSet::new();
    let mut next: Vec<TemplateColumn> = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.iter().enumerate() {
        let base = if input.source >= 1 {
            current.get((input.source - 1) as usize)
        } else {
            None
        };
        let key = match base {
            Some(column) if !used.contains(&column.key) => column.key.clone(),
            _ => next_column_key_from(&used, index + 1),
        };
        used.insert(key.clone());
        next.push(TemplateColumn {
            key,
            label: if input.label.trim().is_empty() {
                format!("列{}", index + 1)
            } else {
                input.label.trim().to_string()
            },
            width: input.width.clamp(6, 80),
            format: input.format.clone(),
            hint: input.hint.clone(),
            options: input.options.clone(),
        });
    }
    let removed: Vec<String> = current
        .iter()
        .filter(|column| !used.contains(&column.key))
        .map(|column| column.key.clone())
        .collect();

    let mut sheets: Vec<TemplateSheet> = template.sheets.clone();
    let Some(target) = sheets.iter_mut().find(|item| item.name == sheet) else {
        return Err(format!("找不到工作表「{}」", sheet));
    };
    target.columns = next;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if !removed.is_empty() {
        scrub_keys(&tx, &template.id, sheet, &removed, &now).map_err(|e| e.to_string())?;
    }
    db::update_template_sheets_in(&tx, &template.id, &sheets).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// 工作表列表与每个表的行数，全部来自数据库。
pub fn list_sheets(
    conn: &Connection,
    template: &TemplateDto,
) -> Result<Vec<SheetInfoDto>, String> {
    let mut out = Vec::with_capacity(template.sheets.len());
    for sheet in &template.sheets {
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM records WHERE template_id = ?1 AND sheet_name = ?2",
                params![template.id, sheet.name],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        out.push(SheetInfoDto {
            name: sheet.name.clone(),
            headers: effective_columns(template, &sheet.name)
                .iter()
                .map(|column| column.label.clone())
                .collect(),
            rows: rows as u32,
        });
    }
    Ok(out)
}

/// 新建工作表：默认带一列「名称」，不做任何 Excel 写入。
pub fn add_sheet(
    conn: &mut Connection,
    template: &TemplateDto,
    name: Option<String>,
) -> Result<String, String> {
    let mut sheets = template.sheets.clone();
    let mut index = sheets.len() + 1;
    let mut candidate = name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("Sheet{}", index));
    while sheets.iter().any(|sheet| sheet.name == candidate) {
        index += 1;
        candidate = format!("Sheet{}", index);
    }
    validate_sheet_name(&candidate)?;
    sheets.push(TemplateSheet {
        name: candidate.clone(),
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
    db::update_template_sheets(conn, &template.id, &sheets).map_err(|e| e.to_string())?;
    Ok(candidate)
}

/// 工作表改名：模板定义和该表所有行的 sheet_name 一起改。
pub fn rename_sheet(
    conn: &mut Connection,
    template: &TemplateDto,
    old_name: &str,
    new_name: &str,
) -> Result<(), String> {
    ensure_sheet(template, old_name)?;
    let new_name = new_name.trim();
    validate_sheet_name(new_name)?;
    if old_name == new_name {
        return Ok(());
    }
    if template.sheets.iter().any(|sheet| sheet.name == new_name) {
        return Err("工作表名称已存在".to_string());
    }
    let mut sheets = template.sheets.clone();
    let Some(target) = sheets.iter_mut().find(|sheet| sheet.name == old_name) else {
        return Err(format!("找不到工作表「{}」", old_name));
    };
    target.name = new_name.to_string();

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    db::update_template_sheets_in(&tx, &template.id, &sheets).map_err(|e| e.to_string())?;
    tx.execute(
        "UPDATE records SET sheet_name = ?1 WHERE template_id = ?2 AND sheet_name = ?3",
        params![new_name, template.id, old_name],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// 删除工作表：有数据时必须显式 force，避免误删整张表。
pub fn delete_sheet(
    conn: &mut Connection,
    template: &TemplateDto,
    name: &str,
    force: bool,
) -> Result<usize, String> {
    ensure_sheet(template, name)?;
    if template.sheets.len() <= 1 {
        return Err("至少保留一个工作表".to_string());
    }
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM records WHERE template_id = ?1 AND sheet_name = ?2",
            params![template.id, name],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if rows > 0 && !force {
        return Err(format!(
            "工作表「{}」还有 {} 行数据，确认删除后再试",
            name, rows
        ));
    }

    let mut sheets = template.sheets.clone();
    sheets.retain(|sheet| sheet.name != name);
    if !sheets.iter().any(|sheet| sheet.primary) {
        if let Some(first) = sheets.first_mut() {
            first.primary = true;
        }
    }

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM records WHERE template_id = ?1 AND sheet_name = ?2",
        params![template.id, name],
    )
    .map_err(|e| e.to_string())?;
    db::update_template_sheets_in(&tx, &template.id, &sheets).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(rows as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TemplateInput;

    fn setup() -> (Connection, TemplateDto) {
        let mut conn = db::test_connection();
        let template = db::test_template(&mut conn);
        (conn, template)
    }

    fn append(
        conn: &mut Connection,
        template: &TemplateDto,
        sheet: &str,
        rows: &[(&str, &str)],
    ) -> Vec<String> {
        let cells: Vec<Vec<String>> = rows
            .iter()
            .map(|(student, subject)| vec![student.to_string(), subject.to_string()])
            .collect();
        append_rows(conn, template, sheet, &cells).unwrap()
    }

    #[test]
    #[ignore]
    fn smoke_real_copy() {
        let db_path = std::env::var("JY_SMOKE_DB").unwrap();
        let mut conn = Connection::open(&db_path).unwrap();
        let templates = db::list_templates(&conn).unwrap();
        let template = templates
            .iter()
            .find(|item| item.id == "tpl_green_channel")
            .unwrap();
        let before = read_rows(&conn, template, Some("sheet1")).unwrap();
        println!("read: rows={} headers={}", before.rows.len(), before.headers.join("/"));
        assert_eq!(before.rows.len(), 17);
        assert_eq!(before.headers.len(), 4);

        let sample_id = before.rows[1].id.clone();
        let sample_json: String = conn
            .query_row(
                "SELECT data_json FROM records WHERE id = ?1",
                params![sample_id],
                |row| row.get(0),
            )
            .unwrap();

        let added = append_rows(
            &mut conn,
            template,
            "sheet1",
            &[vec![
                "冒烟医院".to_string(),
                "冒烟经销商".to_string(),
                "冒烟产品".to_string(),
                "冒烟".to_string(),
            ]],
        )
        .unwrap();
        assert_eq!(added.len(), 1);
        let new_id = added[0].clone();
        let table = read_rows(&conn, template, Some("sheet1")).unwrap();
        assert_eq!(table.rows.len(), 18);
        let last = table.rows.last().unwrap();
        assert_eq!(last.id, new_id);
        assert_eq!(last.cells.join("/"), "冒烟医院/冒烟经销商/冒烟产品/冒烟");
        println!("append: rows={} ok", table.rows.len());

        update_rows(
            &mut conn,
            template,
            "sheet1",
            &[LedgerRowUpdate {
                id: new_id.clone(),
                values: vec![Some("冒烟医院2".to_string())],
            }],
        )
        .unwrap();
        let table = read_rows(&conn, template, Some("sheet1")).unwrap();
        let updated = table.rows.iter().find(|row| row.id == new_id).unwrap();
        assert_eq!(updated.cells[0], "冒烟医院2");
        println!("update: ok");

        let mut order: Vec<String> = table.rows.iter().map(|row| row.id.clone()).collect();
        order.retain(|id| id != &new_id);
        order.insert(0, new_id.clone());
        reorder_rows(&mut conn, template, "sheet1", &order).unwrap();
        let table = read_rows(&conn, template, Some("sheet1")).unwrap();
        assert_eq!(table.rows[0].id, new_id);
        println!("reorder: ok");

        let removed = delete_rows(&mut conn, template, "sheet1", &[new_id.clone()]).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(read_rows(&conn, template, Some("sheet1")).unwrap().rows.len(), 17);
        println!("delete: ok");

        let mut inputs: Vec<SheetColumnInput> = before
            .headers
            .iter()
            .enumerate()
            .map(|(index, label)| SheetColumnInput {
                source: index as u32 + 1,
                label: label.clone(),
                hint: String::new(),
                width: 18,
                format: "text".to_string(),
                options: Vec::new(),
            })
            .collect();
        inputs.push(SheetColumnInput {
            source: 0,
            label: "冒烟备注".to_string(),
            hint: String::new(),
            width: 18,
            format: "text".to_string(),
            options: Vec::new(),
        });
        save_columns(&mut conn, template, "sheet1", &inputs).unwrap();
        let template = db::get_template(&conn, &template.id).unwrap().unwrap();
        let table = read_rows(&conn, &template, Some("sheet1")).unwrap();
        assert_eq!(table.headers.len(), 5);
        assert_eq!(table.headers.last().unwrap(), "冒烟备注");
        println!("column add: headers={}", table.headers.join("/"));

        let restore: Vec<SheetColumnInput> = before
            .headers
            .iter()
            .enumerate()
            .map(|(index, label)| SheetColumnInput {
                source: index as u32 + 1,
                label: label.clone(),
                hint: String::new(),
                width: 18,
                format: "text".to_string(),
                options: Vec::new(),
            })
            .collect();
        save_columns(&mut conn, &template, "sheet1", &restore).unwrap();
        let template = db::get_template(&conn, &template.id).unwrap().unwrap();
        let table = read_rows(&conn, &template, Some("sheet1")).unwrap();
        assert_eq!(table.headers.len(), 4);
        let after_json: String = conn
            .query_row(
                "SELECT data_json FROM records WHERE id = ?1",
                params![sample_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sample_json, after_json);
        println!("column delete: headers={} existing data kept", table.headers.join("/"));

        let sheet = add_sheet(&mut conn, &template, Some("冒烟工作表".to_string())).unwrap();
        let template = db::get_template(&conn, &template.id).unwrap().unwrap();
        append_rows(
            &mut conn,
            &template,
            &sheet,
            &[vec!["临时行".to_string()]],
        )
        .unwrap();
        rename_sheet(&mut conn, &template, &sheet, "冒烟工作表2").unwrap();
        let template = db::get_template(&conn, &template.id).unwrap().unwrap();
        assert_eq!(
            read_rows(&conn, &template, Some("冒烟工作表2"))
                .unwrap()
                .rows
                .len(),
            1
        );
        assert!(delete_sheet(&mut conn, &template, "冒烟工作表2", false).is_err());
        delete_sheet(&mut conn, &template, "冒烟工作表2", true).unwrap();
        let template = db::get_template(&conn, &template.id).unwrap().unwrap();
        assert_eq!(list_sheets(&conn, &template).unwrap().len(), 3);
        println!("sheet add/rename/delete: ok");

        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        println!("integrity: ok");
    }

    #[test]
    fn rows_crud_and_reorder() {
        let (mut conn, template) = setup();
        let ids = append(&mut conn, &template, "台账", &[("张三", "数学"), ("李四", "英语")]);
        assert_eq!(ids.len(), 2);

        let table = read_rows(&conn, &template, None).unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].cells.join("|"), "张三|数学");
        assert_eq!(table.rows[0].row, 1);
        assert_eq!(table.keys.join("|"), "student|subject");

        update_rows(
            &mut conn,
            &template,
            "台账",
            &[LedgerRowUpdate {
                id: ids[0].clone(),
                values: vec![Some("张三丰".to_string()), None],
            }],
        )
        .unwrap();
        let table = read_rows(&conn, &template, None).unwrap();
        assert_eq!(table.rows[0].cells.join("|"), "张三丰|数学");

        // 空字符串表示清空该格，data_json 里不再保留这个 key。
        update_rows(
            &mut conn,
            &template,
            "台账",
            &[LedgerRowUpdate {
                id: ids[0].clone(),
                values: vec![None, Some("".to_string())],
            }],
        )
        .unwrap();
        let table = read_rows(&conn, &template, None).unwrap();
        assert_eq!(table.rows[0].cells.join("|"), "张三丰|");

        reorder_rows(
            &mut conn,
            &template,
            "台账",
            &[ids[1].clone(), ids[0].clone()],
        )
        .unwrap();
        let table = read_rows(&conn, &template, None).unwrap();
        assert_eq!(table.rows[0].cells.join("|"), "李四|英语");

        // 旧 id 再写入必须报错，避免写到别的行上。
        let error = update_rows(
            &mut conn,
            &template,
            "台账",
            &[LedgerRowUpdate {
                id: "rec_missing".to_string(),
                values: vec![Some("x".to_string())],
            }],
        )
        .unwrap_err();
        assert!(error.contains("刷新"));

        let deleted = delete_rows(&mut conn, &template, "台账", &[ids[0].clone()]).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(read_rows(&conn, &template, None).unwrap().rows.len(), 1);
    }

    #[test]
    fn columns_rename_add_and_delete() {
        let (mut conn, template) = setup();
        append(&mut conn, &template, "台账", &[("张三", "数学")]);

        // 交换两列顺序并改名：key 不变，数据跟着列走。
        let inputs = vec![
            SheetColumnInput {
                source: 2,
                label: "课程".to_string(),
                hint: String::new(),
                width: 20,
                format: "text".to_string(),
                options: Vec::new(),
            },
            SheetColumnInput {
                source: 1,
                label: "姓名".to_string(),
                hint: String::new(),
                width: 20,
                format: "text".to_string(),
                options: Vec::new(),
            },
        ];
        save_columns(&mut conn, &template, "台账", &inputs).unwrap();
        let template = db::get_template(&conn, &template.id).unwrap().unwrap();
        let table = read_rows(&conn, &template, None).unwrap();
        assert_eq!(table.headers.join("|"), "课程|姓名");
        assert_eq!(table.keys.join("|"), "subject|student");
        assert_eq!(table.rows[0].cells.join("|"), "数学|张三");

        // 新增一列，key 不能和已有列冲突。
        let inputs = vec![
            SheetColumnInput {
                source: 2,
                label: "课程".to_string(),
                hint: String::new(),
                width: 20,
                format: "text".to_string(),
                options: Vec::new(),
            },
            SheetColumnInput {
                source: 1,
                label: "姓名".to_string(),
                hint: String::new(),
                width: 20,
                format: "text".to_string(),
                options: Vec::new(),
            },
            SheetColumnInput {
                source: 0,
                label: "备注".to_string(),
                hint: String::new(),
                width: 20,
                format: "text".to_string(),
                options: Vec::new(),
            },
        ];
        save_columns(&mut conn, &template, "台账", &inputs).unwrap();
        let template = db::get_template(&conn, &template.id).unwrap().unwrap();
        let table = read_rows(&conn, &template, None).unwrap();
        assert_eq!(table.headers.len(), 3);
        assert!(!table.keys[2].is_empty());
        assert!(!table.keys[..2].contains(&table.keys[2]));

        // 删掉「姓名」列：数据里对应的 key 必须一并清掉。
        let inputs = vec![
            SheetColumnInput {
                source: 2,
                label: "课程".to_string(),
                hint: String::new(),
                width: 20,
                format: "text".to_string(),
                options: Vec::new(),
            },
            SheetColumnInput {
                source: 3,
                label: "备注".to_string(),
                hint: String::new(),
                width: 20,
                format: "text".to_string(),
                options: Vec::new(),
            },
        ];
        save_columns(&mut conn, &template, "台账", &inputs).unwrap();
        let template = db::get_template(&conn, &template.id).unwrap().unwrap();
        let table = read_rows(&conn, &template, None).unwrap();
        assert_eq!(table.headers.join("|"), "课程|备注");
        let raw: String = conn
            .query_row("SELECT data_json FROM records", [], |row| row.get(0))
            .unwrap();
        assert!(!raw.contains("student"));
        assert!(raw.contains("subject"));
    }

    #[test]
    fn headerless_template_uses_excerpt_column() {
        let mut conn = db::test_connection();
        let input = TemplateInput {
            id: Some("tpl_plain".to_string()),
            name: "无表头".to_string(),
            columns: Vec::new(),
            target_path: String::new(),
            sheet_name: "表".to_string(),
            header_row: 1,
            file_name_rule: String::new(),
            is_default: true,
            sheets: vec![TemplateSheet {
                name: "表".to_string(),
                header_row: 1,
                columns: Vec::new(),
                primary: true,
            }],
        };
        let template = db::save_template(&mut conn, &input).unwrap();
        append_rows(
            &mut conn,
            &template,
            "表",
            &[vec!["一段文字".to_string()]],
        )
        .unwrap();
        let table = read_rows(&conn, &template, None).unwrap();
        assert_eq!(table.headers.join("|"), "内容");
        assert_eq!(table.keys.join("|"), "excerpt");
        assert_eq!(table.rows[0].cells.join("|"), "一段文字");
    }

    #[test]
    fn sheets_add_rename_and_delete_with_rows() {
        let (mut conn, template) = setup();
        let name = add_sheet(&mut conn, &template, None).unwrap();
        assert_eq!(name, "Sheet2");
        let template = db::get_template(&conn, &template.id).unwrap().unwrap();
        assert_eq!(list_sheets(&conn, &template).unwrap().len(), 2);

        append(&mut conn, &template, &name, &[("王五", "语文")]);
        let sheets = list_sheets(&conn, &template).unwrap();
        assert_eq!(sheets.iter().find(|sheet| sheet.name == name).unwrap().rows, 1);

        rename_sheet(&mut conn, &template, &name, "第二期").unwrap();
        let template = db::get_template(&conn, &template.id).unwrap().unwrap();
        let sheets = list_sheets(&conn, &template).unwrap();
        assert!(sheets.iter().any(|sheet| sheet.name == "第二期"));
        let table = read_rows(&conn, &template, Some("第二期")).unwrap();
        assert_eq!(table.rows[0].cells[0], "王五");

        // 有数据时不能静默删除。
        assert!(delete_sheet(&mut conn, &template, "第二期", false).is_err());
        let removed = delete_sheet(&mut conn, &template, "第二期", true).unwrap();
        assert_eq!(removed, 1);
        let template = db::get_template(&conn, &template.id).unwrap().unwrap();
        assert_eq!(list_sheets(&conn, &template).unwrap().len(), 1);
    }
}
