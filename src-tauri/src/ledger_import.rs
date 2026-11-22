//! 旧 Excel 台账迁入 SQLite 的对账与导入。
//!
//! 只读 Excel，不改原文件。匹配口径：同一工作表内按模板列顺序把单元格
//! 去掉多余空白后拼接，内容完全相同才算同一行；重复行按出现顺序一一配对。
//! 匹配上的行默认以 Excel 值为准（Excel 可能被人工改过），Excel 独有的行
//! 以 origin=import 追加，只在记录里有的行原样保留。

use crate::db;
use crate::ledger;
use crate::models::{TemplateColumn, TemplateDto, TemplateSheet};
use crate::{pad_template_columns, sheet_columns};
use rusqlite::{params, Connection};
use serde_json::{Map, Value};
use std::collections::{HashMap, VecDeque};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct SheetCounts {
    pub name: String,
    pub excel_rows: i64,
    pub matched: i64,
    pub excel_only: i64,
    pub record_only: i64,
    pub updated: i64,
}

struct ExistingRow {
    id: String,
    data: Map<String, Value>,
    cells: Vec<String>,
}

pub struct SheetPlan {
    pub name: String,
    pub columns: Vec<TemplateColumn>,
    pub added_columns: Vec<String>,
    pub counts: SheetCounts,
    updates: Vec<(String, Value)>,
    imports: Vec<Vec<String>>,
}

pub struct ImportPlan {
    pub workbook: String,
    pub sheets: Vec<SheetPlan>,
    pub ignored_sheets: Vec<String>,
}

impl ImportPlan {
    /// (Excel 行数, 匹配行数, Excel 独有, 仅记录有, 值不同)
    pub fn totals(&self) -> (i64, i64, i64, i64, i64) {
        self.sheets.iter().fold(
            (0, 0, 0, 0, 0),
            |(excel, matched, excel_only, record_only, updated), sheet| {
                (
                    excel + sheet.counts.excel_rows,
                    matched + sheet.counts.matched,
                    excel_only + sheet.counts.excel_only,
                    record_only + sheet.counts.record_only,
                    updated + sheet.counts.updated,
                )
            },
        )
    }

    pub fn columns_added(&self) -> Vec<String> {
        self.sheets
            .iter()
            .flat_map(|sheet| sheet.added_columns.iter().cloned())
            .collect()
    }
}

pub struct ApplyOutcome {
    pub inserted: i64,
    pub updated: i64,
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn row_key(cells: &[String]) -> String {
    cells
        .iter()
        .map(|cell| normalize(cell))
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

fn cells_from_data(columns: &[TemplateColumn], data: &Map<String, Value>) -> Vec<String> {
    columns
        .iter()
        .map(|column| {
            data.get(&column.key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        })
        .collect()
}

/// 以现有数据为底，按列写入单元格；空单元格会移除对应 key，保留其它未知字段。
fn merge_data(
    existing: &Map<String, Value>,
    columns: &[TemplateColumn],
    cells: &[String],
) -> Map<String, Value> {
    let mut data = existing.clone();
    for (index, column) in columns.iter().enumerate() {
        let value = cells
            .get(index)
            .map(|cell| cell.trim())
            .unwrap_or("");
        if value.is_empty() {
            data.remove(&column.key);
        } else {
            data.insert(column.key.clone(), Value::String(value.to_string()));
        }
    }
    data
}

fn derive_columns(headers: &[String]) -> Vec<TemplateColumn> {
    headers
        .iter()
        .enumerate()
        .map(|(index, header)| TemplateColumn {
            key: format!("col_{}", index + 1),
            label: if header.trim().is_empty() {
                format!("列{}", index + 1)
            } else {
                header.trim().to_string()
            },
            width: 18,
            format: "text".to_string(),
            hint: String::new(),
            options: Vec::new(),
        })
        .collect()
}

fn resolve_sheet_name(
    template_name: &str,
    workbook_names: &[String],
    template_count: usize,
) -> Option<String> {
    if let Some(name) = workbook_names.iter().find(|name| *name == template_name) {
        return Some(name.clone());
    }
    if let Some(name) = workbook_names
        .iter()
        .find(|name| name.eq_ignore_ascii_case(template_name))
    {
        return Some(name.clone());
    }
    if template_count == 1 && workbook_names.len() == 1 {
        return Some(workbook_names[0].clone());
    }
    None
}

/// 业务键：第一列（通常是姓名/名称）非空时作为兜底匹配依据。
fn first_key(cells: &[String]) -> String {
    cells
        .first()
        .map(|cell| normalize(cell))
        .unwrap_or_default()
}

/// 先按整行内容精确配对；配对不上的再按第一列唯一匹配，
/// 这样客户在 Excel 里改过某个单元格时会被识别为“覆盖”而不是新增重复行。
fn reconcile(
    rows: &[Vec<String>],
    existing: Vec<ExistingRow>,
    columns: &[TemplateColumn],
    counts: &mut SheetCounts,
) -> (Vec<(String, Value)>, Vec<Vec<String>>) {
    use std::collections::HashSet;

    let mut by_id: HashMap<String, ExistingRow> = HashMap::new();
    let mut exact: HashMap<String, VecDeque<String>> = HashMap::new();
    let mut by_first: HashMap<String, Vec<String>> = HashMap::new();
    for row in existing {
        let id = row.id.clone();
        exact
            .entry(row_key(&row.cells))
            .or_default()
            .push_back(id.clone());
        let first = first_key(&row.cells);
        if !first.is_empty() {
            by_first.entry(first).or_default().push(id.clone());
        }
        by_id.insert(id, row);
    }

    let mut consumed: HashSet<String> = HashSet::new();
    let mut pending: Vec<&Vec<String>> = Vec::new();
    for cells in rows {
        let key = row_key(cells);
        match exact.get_mut(&key).and_then(|queue| queue.pop_front()) {
            Some(id) => {
                consumed.insert(id);
                counts.matched += 1;
            }
            None => pending.push(cells),
        }
    }

    let mut updates = Vec::new();
    let mut imports = Vec::new();
    for cells in pending {
        let first = first_key(cells);
        let candidate = if first.is_empty() {
            None
        } else {
            by_first
                .get(&first)
                .map(|ids| {
                    ids.iter()
                        .filter(|id| !consumed.contains(*id))
                        .collect::<Vec<_>>()
                })
                .filter(|ids| ids.len() == 1)
                .map(|ids| ids[0].to_string())
        };
        match candidate {
            Some(id) => {
                consumed.insert(id.clone());
                counts.matched += 1;
                let existing = &by_id[&id];
                let merged = merge_data(&existing.data, columns, cells);
                if merged != existing.data {
                    counts.updated += 1;
                    updates.push((id, Value::Object(merged)));
                }
            }
            None => {
                counts.excel_only += 1;
                imports.push(cells.clone());
            }
        }
    }
    counts.record_only = (by_id.len() - consumed.len()) as i64;
    (updates, imports)
}

fn load_existing(
    conn: &Connection,
    template_id: &str,
    sheet_name: &str,
    columns: &[TemplateColumn],
) -> Result<Vec<ExistingRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, data_json FROM records
              WHERE template_id = ?1 AND sheet_name = ?2
              ORDER BY sort_order ASC, id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![template_id, sheet_name], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, raw) = row.map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Object(Map::new()));
        let data = value.as_object().cloned().unwrap_or_default();
        let cells = cells_from_data(columns, &data);
        out.push(ExistingRow { id, data, cells });
    }
    Ok(out)
}

fn import_title(cells: &[String], index: usize) -> String {
    let text = cells
        .iter()
        .map(|cell| cell.trim())
        .find(|cell| !cell.is_empty())
        .unwrap_or("");
    if text.is_empty() {
        format!("导入行 {}", index + 1)
    } else {
        text.chars().take(60).collect()
    }
}

/// 只读对账：算出每个工作表要新增、要覆盖、只在记录里存在的行数。
pub fn build_plan(
    conn: &Connection,
    template: &TemplateDto,
    path: &Path,
) -> Result<ImportPlan, String> {
    if !path.exists() {
        return Err(format!("找不到要导入的台账文件：{}", path.display()));
    }
    let workbook = ledger::read_workbook(path)?;
    let workbook_names: Vec<String> = workbook.iter().map(|sheet| sheet.name.clone()).collect();
    let mut used: Vec<String> = Vec::new();
    let mut sheets = Vec::new();

    for template_sheet in &template.sheets {
        let Some(actual) =
            resolve_sheet_name(&template_sheet.name, &workbook_names, template.sheets.len())
        else {
            continue;
        };
        if used.iter().any(|name| name.eq_ignore_ascii_case(&actual)) {
            continue;
        }
        used.push(actual.clone());

        let header_row = template_sheet.header_row.max(1) as u32;
        let table = ledger::read_sheet_table(path, &actual, header_row)?;
        let mut columns = sheet_columns(template, &template_sheet.name);
        if columns.is_empty() {
            columns = derive_columns(&table.headers);
        }
        let before = columns.len();
        pad_template_columns(&mut columns, &table.headers);
        let added_columns: Vec<String> = columns[before..]
            .iter()
            .map(|column| column.label.clone())
            .collect();

        let width = columns.len();
        let rows: Vec<Vec<String>> = table
            .rows
            .iter()
            .map(|row| {
                let mut cells = vec![String::new(); width];
                for (index, cell) in row.cells.iter().enumerate().take(width) {
                    cells[index] = cell.clone();
                }
                cells
            })
            .filter(|cells| cells.iter().any(|cell| !cell.trim().is_empty()))
            .collect();

        let existing = load_existing(conn, &template.id, &template_sheet.name, &columns)?;
        let mut counts = SheetCounts {
            name: template_sheet.name.clone(),
            excel_rows: rows.len() as i64,
            ..SheetCounts::default()
        };
        let (updates, imports) = reconcile(&rows, existing, &columns, &mut counts);

        sheets.push(SheetPlan {
            name: template_sheet.name.clone(),
            columns,
            added_columns,
            counts,
            updates,
            imports,
        });
    }

    let ignored_sheets = workbook_names
        .iter()
        .filter(|name| !used.iter().any(|used| used.eq_ignore_ascii_case(name)))
        .cloned()
        .collect();

    Ok(ImportPlan {
        workbook: path.to_string_lossy().to_string(),
        sheets,
        ignored_sheets,
    })
}

/// 落库：先补齐模板列定义，再在同一事务里覆盖/插入记录。
/// `policy` 为 "record" 时保留记录原值，其余取值都以 Excel 为准。
pub fn apply(
    conn: &mut Connection,
    template: &TemplateDto,
    path: &Path,
    policy: &str,
) -> Result<(ImportPlan, ApplyOutcome), String> {
    let plan = build_plan(conn, template, path)?;

    let mut sheets: Vec<TemplateSheet> = template.sheets.clone();
    let mut columns_changed = false;
    for sheet in &plan.sheets {
        if sheet.added_columns.is_empty() {
            continue;
        }
        if let Some(target) = sheets.iter_mut().find(|item| item.name == sheet.name) {
            target.columns = sheet.columns.clone();
            columns_changed = true;
        }
    }
    if columns_changed {
        db::update_template_sheets(conn, &template.id, &sheets).map_err(|e| e.to_string())?;
    }

    let prefer_excel = policy != "record";
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut outcome = ApplyOutcome {
        inserted: 0,
        updated: 0,
    };

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for sheet in &plan.sheets {
        if prefer_excel {
            for (id, data) in &sheet.updates {
                let json = serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string());
                tx.execute(
                    "UPDATE records SET data_json = ?1, updated_at = ?2 WHERE id = ?3",
                    params![json, now, id],
                )
                .map_err(|e| e.to_string())?;
                outcome.updated += 1;
            }
        }

        let base: f64 = tx
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM records
                  WHERE template_id = ?1 AND sheet_name = ?2",
                params![template.id, sheet.name],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        for (index, cells) in sheet.imports.iter().enumerate() {
            let data = merge_data(&Map::new(), &sheet.columns, cells);
            let json =
                serde_json::to_string(&Value::Object(data)).unwrap_or_else(|_| "{}".to_string());
            tx.execute(
                "INSERT INTO records
                 (id, group_id, title, data_json, template_id, sheet_name, sort_order, origin, created_at, updated_at)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, 'import', ?7, ?7)",
                params![
                    db::next_id("rec"),
                    import_title(cells, index),
                    json,
                    template.id,
                    sheet.name,
                    base + index as f64 + 1.0,
                    now
                ],
            )
            .map_err(|e| e.to_string())?;
            outcome.inserted += 1;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;

    Ok((plan, outcome))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("jy-import-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn workbook(dir: &Path, rows: &[(&str, &str)]) -> std::path::PathBuf {
        let path = dir.join("台账.xlsx");
        let headers = vec!["学员".to_string(), "科目".to_string()];
        ledger::create_workbook_with_sheets(&path, &[("台账".to_string(), headers.clone())])
            .unwrap();
        let cells: Vec<Vec<ledger::CellValue>> = rows
            .iter()
            .map(|(student, subject)| {
                vec![
                    ledger::CellValue::Text(student.to_string()),
                    ledger::CellValue::Text(subject.to_string()),
                ]
            })
            .collect();
        ledger::append_to_workbook(&path, "台账", 1, &headers, &cells).unwrap();
        path
    }

    #[test]
    fn preview_counts_and_excel_wins() {
        let dir = temp_dir("preview");
        let mut conn = db::test_connection();
        let template = db::test_template(&mut conn);
        let path = workbook(&dir, &[("张三", "数学"), ("李四", "英语")]);

        let payload = crate::models::RecordPayload {
            group_id: "g1".to_string(),
            title: "张三".to_string(),
            data: serde_json::json!({ "student": "张三", "subject": "数学" }),
            sources: Vec::new(),
            template_id: Some(template.id.clone()),
        };
        db::create_record(&mut conn, &payload).unwrap();
        let payload = crate::models::RecordPayload {
            group_id: "g1".to_string(),
            title: "王五".to_string(),
            data: serde_json::json!({ "student": "王五", "subject": "语文" }),
            sources: Vec::new(),
            template_id: Some(template.id.clone()),
        };
        db::create_record(&mut conn, &payload).unwrap();

        let plan = build_plan(&conn, &template, &path).unwrap();
        assert_eq!(plan.totals(), (2, 1, 1, 1, 0));

        let outcome = apply(&mut conn, &template, &path, "excel").unwrap().1;
        assert_eq!(outcome.inserted, 1);
        assert_eq!(outcome.updated, 0);
        let records = db::list_records(&conn).unwrap();
        assert_eq!(records.len(), 3);
        assert!(records
            .iter()
            .any(|record| record.origin == "import"
                && record.data.get("student").and_then(Value::as_str) == Some("李四")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn matched_row_follows_excel_value() {
        let dir = temp_dir("update");
        let mut conn = db::test_connection();
        let template = db::test_template(&mut conn);
        let path = workbook(&dir, &[("张三", "物理")]);

        let payload = crate::models::RecordPayload {
            group_id: "g1".to_string(),
            title: "张三".to_string(),
            data: serde_json::json!({ "student": "张三", "subject": "数学" }),
            sources: Vec::new(),
            template_id: Some(template.id.clone()),
        };
        db::create_record(&mut conn, &payload).unwrap();

        // 第一列相同、内容被改过：算匹配并覆盖，不算新增重复行。
        let plan = build_plan(&conn, &template, &path).unwrap();
        assert_eq!(plan.totals(), (1, 1, 0, 0, 1));

        let outcome = apply(&mut conn, &template, &path, "excel").unwrap().1;
        assert_eq!(outcome.updated, 1);
        assert_eq!(outcome.inserted, 0);
        let records = db::list_records(&conn).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].data.get("subject").and_then(Value::as_str),
            Some("物理")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
