use crate::db;
use crate::models::RecordDto;
use rusqlite::Connection;
use rust_xlsxwriter::{Color, Format, FormatAlign, Workbook};

/// 「另存为」：导出一个独立文件，不改动原台账，也不改变记录的已导出状态。
pub fn export_records(
    conn: &Connection,
    ids: &[String],
    template_id: &str,
    path: &str,
) -> Result<usize, String> {
    let all = db::list_records(conn).map_err(|e| e.to_string())?;
    let selected: Vec<RecordDto> = if ids.is_empty() {
        all
    } else {
        all.into_iter()
            .filter(|record| ids.iter().any(|id| id == &record.id))
            .collect()
    };
    if selected.is_empty() {
        return Err("没有可导出的记录".to_string());
    }

    let template = db::get_template(conn, template_id)
        .map_err(|e| e.to_string())?
        .or_else(|| {
            db::list_templates(conn)
                .ok()
                .and_then(|list| list.into_iter().next())
        });
    let columns = template
        .as_ref()
        .map(|template| template.columns.clone())
        .unwrap_or_default();
    let headerless = columns.is_empty();

    let target = std::path::Path::new(path);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err("目标文件夹不存在，请重新选择保存位置".to_string());
        }
    }
    if template
        .as_ref()
        .map(|item| {
            let raw = item.target_path.trim();
            raw.to_lowercase().ends_with(".xlsx") && raw == path
        })
        .unwrap_or(false)
    {
        return Err("不能覆盖当前追加台账，请换一个文件名".to_string());
    }

    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    let sheet_name = template
        .as_ref()
        .map(|template| template.sheet_name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "sheet1".to_string());
    sheet.set_name(&sheet_name).map_err(|e| e.to_string())?;

    let header_format = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xEDEDEB))
        .set_font_color(Color::RGB(0x1A1A1A))
        .set_align(FormatAlign::Center);
    let wrap_format = Format::new().set_text_wrap();

    for (col, column) in columns.iter().enumerate() {
        sheet
            .write_with_format(0, col as u16, &column.label, &header_format)
            .map_err(|e| e.to_string())?;
        sheet
            .set_column_width(col as u16, column.width.clamp(6, 80) as f64)
            .map_err(|e| e.to_string())?;
    }

    if headerless {
        for (row_index, record) in selected.iter().enumerate() {
            let text = record
                .data
                .get("excerpt")
                .and_then(|value| value.as_str())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| record.title.clone());
            sheet
                .write_with_format(row_index as u32, 0, text, &wrap_format)
                .map_err(|e| e.to_string())?;
        }
        sheet.set_column_width(0, 60).map_err(|e| e.to_string())?;
    } else {

    for (row_index, record) in selected.iter().enumerate() {
        let row = (row_index + 1) as u32;
        for (col, column) in columns.iter().enumerate() {
            let value = record
                .data
                .get(&column.key)
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if value.is_empty() {
                continue;
            }
            if column.format == "number" {
                if let Ok(number) = value.parse::<f64>() {
                    sheet
                        .write_number(row, col as u16, number)
                        .map_err(|e| e.to_string())?;
                    continue;
                }
            }
            if column.key == "excerpt" {
                sheet
                    .write_with_format(row, col as u16, value, &wrap_format)
                    .map_err(|e| e.to_string())?;
            } else {
                sheet
                    .write_string(row, col as u16, value)
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    }

    sheet.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;
    workbook
        .save(path)
        .map_err(|e| format!("保存失败：{}（如文件正在 Excel 中打开，请先关闭）", e))?;
    Ok(selected.len())
}
