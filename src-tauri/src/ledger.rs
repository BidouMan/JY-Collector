//! 追加写入现有 xlsx 工作簿。
//!
//! 为什么不用「读出全部数据再整体重写」：那样会丢掉原台账里的样式、公式和
//! 其它工作表。这里直接对 xlsx（本质是 zip）做定点修改：
//! 只重写目标工作表的 XML，在原 `<sheetData>` 末尾插入新行，缺列时在表头
//! 行末尾补列；其它 zip 条目原样复制。最后写临时文件再原子替换，避免写坏原文件。

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Clone)]
pub enum CellValue {
    Text(String),
    Number(f64),
}

impl CellValue {
    fn is_empty(&self) -> bool {
        matches!(self, CellValue::Text(value) if value.trim().is_empty())
    }
}

pub struct AppendReport {
    pub written: usize,
    pub added_columns: Vec<String>,
    pub sheet: String,
}

struct ParsedCell {
    col: u32,
    text: String,
    style: Option<String>,
}

struct ParsedRow {
    index: u32,
    cells: Vec<ParsedCell>,
}

/// 按表头列名匹配后追加；返回实际写入行数、补出来的列名和命中的工作表。
pub fn append_to_workbook(
    path: &Path,
    sheet_name: &str,
    header_row: u32,
    headers: &[String],
    rows: &[Vec<CellValue>],
) -> Result<AppendReport, String> {
    let header_row = header_row.max(1);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(format!("目标目录不存在：{}", parent.display()));
        }
    }
    if !path.exists() {
        create_workbook(path, sheet_name, headers, rows)?;
        return Ok(AppendReport {
            written: rows.len(),
            added_columns: Vec::new(),
            sheet: if sheet_name.is_empty() {
                "Sheet1".to_string()
            } else {
                sheet_name.to_string()
            },
        });
    }
    if path.is_dir() {
        return Err("目标路径是一个文件夹，请选择 xlsx 文件".to_string());
    }
    if excel_lock_exists(path) {
        return Err("目标文件正在 Excel 中打开，请先关闭后再导出".to_string());
    }
    let meta = fs::metadata(path).map_err(|e| format!("无法读取目标文件：{}", e))?;
    if meta.permissions().readonly() {
        return Err("目标文件是只读的，请取消只读后再导出".to_string());
    }

    let file = fs::File::open(path).map_err(|e| format!("无法打开目标文件：{}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| "目标文件不是有效的 xlsx 工作簿".to_string())?;

    let workbook_xml =
        read_entry(&mut archive, "xl/workbook.xml").ok_or("工作簿缺少 xl/workbook.xml")?;
    let rels_xml = read_entry(&mut archive, "xl/_rels/workbook.xml.rels")
        .ok_or("工作簿缺少关系定义")?;
    let sheets = parse_sheets(&workbook_xml);
    if sheets.is_empty() {
        return Err("工作簿里没有工作表".to_string());
    }
    let rels = parse_rels(&rels_xml);
    let wanted = sheet_name.trim();
    let (matched_name, part) = if wanted.is_empty() {
        let (name, rid) = &sheets[0];
        (name.clone(), rel_target(rid, &rels)?)
    } else {
        match sheets.iter().find(|(name, _)| name == wanted) {
            Some((name, rid)) => (name.clone(), rel_target(rid, &rels)?),
            None if sheets.len() == 1 => {
                let (name, rid) = &sheets[0];
                (name.clone(), rel_target(rid, &rels)?)
            }
            None => {
                return Err(format!(
                    "工作簿中没有工作表「{}」，请先在模板里改成已有工作表名",
                    wanted
                ));
            }
        }
    };

    let shared = read_entry(&mut archive, "xl/sharedStrings.xml")
        .map(|xml| parse_shared_strings(&xml))
        .unwrap_or_default();
    let sheet_xml = read_entry(&mut archive, &part)
        .ok_or_else(|| format!("工作表「{}」的数据缺失", matched_name))?;
    let parsed = parse_sheet(&sheet_xml, &shared);

    let max_row = parsed.iter().map(|row| row.index).max().unwrap_or(0);
    let max_col = parsed
        .iter()
        .flat_map(|row| row.cells.iter().map(|cell| cell.col))
        .max()
        .unwrap_or(0);

    let header_cells: Vec<&ParsedCell> = parsed
        .iter()
        .find(|row| row.index == header_row)
        .map(|row| row.cells.iter().collect())
        .unwrap_or_default();

    if header_cells.is_empty() && max_row > 0 {
        return Err(format!(
            "工作表「{}」第 {} 行没有表头，请先在模板里调整表头行",
            matched_name, header_row
        ));
    }

    let header_style = header_cells
        .iter()
        .find_map(|cell| cell.style.clone());

    let mut existing: HashMap<String, u32> = HashMap::new();
    for cell in &header_cells {
        let label = cell.text.trim();
        if !label.is_empty() {
            existing.entry(label.to_string()).or_insert(cell.col);
        }
    }

    let mut next_col = max_col + 1;
    let mut added_columns: Vec<String> = Vec::new();
    let mut col_for: Vec<u32> = Vec::with_capacity(headers.len());
    for header in headers {
        let label = header.trim().to_string();
        if let Some(col) = existing.get(&label) {
            col_for.push(*col);
        } else {
            col_for.push(next_col);
            added_columns.push(label);
            next_col += 1;
        }
    }

    let start_row = if header_cells.is_empty() {
        header_row + 1
    } else {
        max_row.max(header_row) + 1
    };

    let mut new_max_col = max_col;
    let mut new_max_row = max_row;
    let mut base = sheet_xml.clone();
    let mut header_block = String::new();

    if header_cells.is_empty() {
        // 空工作表：先补一整行表头。
        let mut cells = String::new();
        for (index, header) in headers.iter().enumerate() {
            let col = index as u32 + 1;
            new_max_col = new_max_col.max(col);
            cells.push_str(&cell_xml(
                col,
                header_row,
                &CellValue::Text(header.clone()),
                header_style.as_deref(),
            ));
        }
        header_block = format!("<row r=\"{}\">{}</row>", header_row, cells);
        new_max_row = new_max_row.max(header_row);
    } else {
        // 在表头行末尾补上缺失的列。
        let mut cells = String::new();
        for (index, header) in headers.iter().enumerate() {
            if existing.contains_key(header.trim()) {
                continue;
            }
            let col = col_for[index];
            new_max_col = new_max_col.max(col);
            cells.push_str(&cell_xml(
                col,
                header_row,
                &CellValue::Text(header.clone()),
                header_style.as_deref(),
            ));
        }
        if !cells.is_empty() {
            base = insert_header_cells(&sheet_xml, header_row, &cells)?;
        }
    }

    let mut rows_xml = String::new();
    for (offset, row) in rows.iter().enumerate() {
        let row_index = start_row + offset as u32;
        new_max_row = new_max_row.max(row_index);
        let mut cells: Vec<(u32, String)> = Vec::new();
        for (index, value) in row.iter().enumerate() {
            if value.is_empty() {
                continue;
            }
            let col = *col_for.get(index).ok_or("列数与模板不一致")?;
            new_max_col = new_max_col.max(col);
            cells.push((col, cell_xml(col, row_index, value, None)));
        }
        cells.sort_by_key(|(col, _)| *col);
        let cells: String = cells.into_iter().map(|(_, xml)| xml).collect();
        rows_xml.push_str(&format!("<row r=\"{}\">{}</row>", row_index, cells));
    }

    let mut updated =
        insert_before_sheet_data_end(&base, &format!("{}{}", header_block, rows_xml))?;

    if new_max_row > 0 && new_max_col > 0 {
        updated = update_dimension(&updated, new_max_row, new_max_col);
    }

    write_archive(path, &part, &updated)?;

    Ok(AppendReport {
        written: rows.len(),
        added_columns,
        sheet: matched_name,
    })
}

/// 工作表里已有表头但缺列时，把新表头单元格插到该行末尾。
fn insert_header_cells(xml: &str, header_row: u32, cells: &str) -> Result<String, String> {
    let (start, end, self_closing) = find_row_tag(xml, header_row)
        .ok_or_else(|| format!("没找到第 {} 行表头", header_row))?;
    let mut out = String::with_capacity(xml.len() + cells.len() + 16);
    if self_closing {
        let tag = &xml[start..end];
        let open = tag.trim_end_matches("/>");
        out.push_str(&xml[..start]);
        out.push_str(open);
        out.push('>');
        out.push_str(cells);
        out.push_str("</row>");
        out.push_str(&xml[end..]);
    } else {
        let close = xml[end..]
            .find("</row>")
            .map(|offset| end + offset)
            .ok_or_else(|| format!("第 {} 行表头结构异常", header_row))?;
        out.push_str(&xml[..close]);
        out.push_str(cells);
        out.push_str(&xml[close..]);
    }
    Ok(out)
}

fn insert_before_sheet_data_end(xml: &str, rows: &str) -> Result<String, String> {
    if let Some(pos) = xml.find("</sheetData>") {
        let mut out = String::with_capacity(xml.len() + rows.len());
        out.push_str(&xml[..pos]);
        out.push_str(rows);
        out.push_str(&xml[pos..]);
        return Ok(out);
    }
    if let Some(pos) = xml.find("<sheetData/>") {
        let mut out = String::with_capacity(xml.len() + rows.len() + 13);
        out.push_str(&xml[..pos]);
        out.push_str("<sheetData>");
        out.push_str(rows);
        out.push_str("</sheetData>");
        out.push_str(&xml[pos + "<sheetData/>".len()..]);
        return Ok(out);
    }
    Err("工作表结构异常，找不到 sheetData".to_string())
}

fn cell_xml(col: u32, row: u32, value: &CellValue, style: Option<&str>) -> String {
    let reference = format!("{}{}", index_to_col(col), row);
    let style_attr = style
        .map(|value| format!(" s=\"{}\"", value))
        .unwrap_or_default();
    match value {
        CellValue::Number(number) => {
            let text = if number.fract() == 0.0 {
                format!("{}", *number as i64)
            } else {
                format!("{}", number)
            };
            format!("<c r=\"{}\"{}><v>{}</v></c>", reference, style_attr, text)
        }
        CellValue::Text(text) => format!(
            "<c r=\"{}\"{} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
            reference,
            style_attr,
            escape_xml(text)
        ),
    }
}

fn empty_cell_xml(col: u32, row: u32, style: Option<&str>) -> String {
    let reference = format!("{}{}", index_to_col(col), row);
    match style {
        Some(style) => format!("<c r=\"{}\" s=\"{}\"/>", reference, style),
        None => format!("<c r=\"{}\"/>", reference),
    }
}

fn update_dimension(xml: &str, max_row: u32, max_col: u32) -> String {
    let Some(start) = xml.find("<dimension") else {
        return xml.to_string();
    };
    let Some(rel_end) = xml[start..].find("/>") else {
        return xml.to_string();
    };
    let end = start + rel_end + 2;
    let tag = &xml[start..end];
    let reference = format!("A1:{}{}", index_to_col(max_col), max_row);
    let new_tag = if let Some(ref_start) = tag.find("ref=\"") {
        let value_start = start + ref_start + 5;
        let Some(value_end_rel) = xml[value_start..].find('"') else {
            return xml.to_string();
        };
        let value_end = value_start + value_end_rel;
        let mut out = String::with_capacity(xml.len());
        out.push_str(&xml[..value_start]);
        out.push_str(&reference);
        out.push_str(&xml[value_end..]);
        out
    } else {
        let replacement = format!("<dimension ref=\"{}\"/>", reference);
        let mut out = String::with_capacity(xml.len());
        out.push_str(&xml[..start]);
        out.push_str(&replacement);
        out.push_str(&xml[end..]);
        out
    };
    new_tag
}

fn create_workbook(
    path: &Path,
    sheet_name: &str,
    headers: &[String],
    rows: &[Vec<CellValue>],
) -> Result<(), String> {
    use rust_xlsxwriter::{Color, Format, FormatAlign, Workbook};

    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    let name = if sheet_name.trim().is_empty() {
        "Sheet1"
    } else {
        sheet_name.trim()
    };
    sheet.set_name(name).map_err(|e| e.to_string())?;

    let header_format = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xEDEDEB))
        .set_font_color(Color::RGB(0x1A1A1A))
        .set_align(FormatAlign::Center);

    for (col, header) in headers.iter().enumerate() {
        sheet
            .write_with_format(0, col as u16, header, &header_format)
            .map_err(|e| e.to_string())?;
        sheet
            .set_column_width(col as u16, 18)
            .map_err(|e| e.to_string())?;
    }
    for (row_index, row) in rows.iter().enumerate() {
        for (col, value) in row.iter().enumerate() {
            let row = (row_index + 1) as u32;
            match value {
                CellValue::Number(number) => {
                    sheet
                        .write_number(row, col as u16, *number)
                        .map_err(|e| e.to_string())?;
                }
                CellValue::Text(text) => {
                    if !text.is_empty() {
                        sheet
                            .write_string(row, col as u16, text)
                            .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }
    if !headers.is_empty() {
        sheet
            .set_freeze_panes(1, 0)
            .map_err(|e| e.to_string())?;
    }
    workbook
        .save(path)
        .map_err(|e| format!("创建台账文件失败：{}", e))?;
    Ok(())
}

/// 新建一个只含指定工作表、无表头无数据的空工作簿。
pub fn create_empty_workbook(path: &Path, sheet_name: &str) -> Result<(), String> {
    create_workbook(path, sheet_name, &[], &[])
}

#[derive(Debug, Clone)]
pub struct SheetInfo {
    pub name: String,
    pub headers: Vec<String>,
    pub rows: u32,
}

/// 读取工作簿的所有工作表及每个 sheet 的首行表头。
pub fn read_workbook(path: &Path) -> Result<Vec<SheetInfo>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path).map_err(|e| format!("无法打开目标文件：{}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| "目标文件不是有效的 xlsx 工作簿".to_string())?;
    let workbook_xml =
        read_entry(&mut archive, "xl/workbook.xml").ok_or("工作簿缺少 xl/workbook.xml")?;
    let rels_xml =
        read_entry(&mut archive, "xl/_rels/workbook.xml.rels").ok_or("工作簿缺少关系定义")?;
    let sheets = parse_sheets(&workbook_xml);
    let rels = parse_rels(&rels_xml);
    let shared = read_entry(&mut archive, "xl/sharedStrings.xml")
        .map(|xml| parse_shared_strings(&xml))
        .unwrap_or_default();

    let mut out = Vec::new();
    for (name, rid) in sheets {
        let part = rel_target(&rid, &rels)?;
        let xml = read_entry(&mut archive, &part).unwrap_or_default();
        let parsed = parse_sheet(&xml, &shared);
        let rows = parsed.iter().map(|row| row.index).max().unwrap_or(0);
        let first = parsed.iter().min_by_key(|row| row.index);
        let headers = first
            .map(|row| {
                let max_col = row.cells.iter().map(|cell| cell.col).max().unwrap_or(0);
                let mut cells = vec![String::new(); max_col as usize];
                for cell in &row.cells {
                    if cell.col >= 1 {
                        cells[(cell.col - 1) as usize] = cell.text.trim().to_string();
                    }
                }
                while cells.last().map(|value| value.is_empty()).unwrap_or(false) {
                    cells.pop();
                }
                cells
            })
            .unwrap_or_default();
        out.push(SheetInfo { name, headers, rows });
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct SheetRowData {
    pub index: u32,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SheetTable {
    pub name: String,
    pub headers: Vec<String>,
    pub rows: Vec<SheetRowData>,
}

struct SheetXml {
    part: String,
    name: String,
    xml: String,
    shared: Vec<String>,
}

struct RowCellSpan {
    col: u32,
    start: usize,
    end: usize,
    style: Option<String>,
}

fn load_sheet_xml(path: &Path, sheet_name: &str) -> Result<SheetXml, String> {
    let file = fs::File::open(path).map_err(|e| format!("无法打开目标文件：{}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| "目标文件不是有效的 xlsx 工作簿".to_string())?;
    let workbook_xml =
        read_entry(&mut archive, "xl/workbook.xml").ok_or("工作簿缺少 xl/workbook.xml")?;
    let rels_xml =
        read_entry(&mut archive, "xl/_rels/workbook.xml.rels").ok_or("工作簿缺少关系定义")?;
    let sheets = parse_sheets(&workbook_xml);
    if sheets.is_empty() {
        return Err("工作簿里没有工作表".to_string());
    }
    let rels = parse_rels(&rels_xml);
    let wanted = sheet_name.trim();
    let (name, rid) = if wanted.is_empty() {
        sheets[0].clone()
    } else {
        match sheets.iter().find(|(name, _)| name == wanted) {
            Some((name, rid)) => (name.clone(), rid.clone()),
            None if sheets.len() == 1 => sheets[0].clone(),
            None => return Err(format!("工作簿中没有工作表「{}」", wanted)),
        }
    };
    let part = rel_target(&rid, &rels)?;
    let xml = read_entry(&mut archive, &part)
        .ok_or_else(|| format!("工作表「{}」的数据缺失", name))?;
    let shared = read_entry(&mut archive, "xl/sharedStrings.xml")
        .map(|value| parse_shared_strings(&value))
        .unwrap_or_default();
    Ok(SheetXml {
        part,
        name,
        xml,
        shared,
    })
}

fn check_writable(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("表格文件不存在：{}", path.display()));
    }
    if path.is_dir() {
        return Err("目标路径是一个文件夹，请选择 xlsx 文件".to_string());
    }
    if excel_lock_exists(path) {
        return Err("目标文件正在 Excel 中打开，请先关闭后再修改".to_string());
    }
    let meta = fs::metadata(path).map_err(|e| format!("无法读取目标文件：{}", e))?;
    if meta.permissions().readonly() {
        return Err("目标文件是只读的，请取消只读后再修改".to_string());
    }
    Ok(())
}

fn max_row_col(rows: &[ParsedRow]) -> (u32, u32) {
    let max_row = rows.iter().map(|row| row.index).max().unwrap_or(0);
    let max_col = rows
        .iter()
        .flat_map(|row| row.cells.iter().map(|cell| cell.col))
        .max()
        .unwrap_or(0);
    (max_row, max_col)
}

/// 读出整张工作表：表头按列对齐，数据行保留 Excel 里的真实行号。
pub fn read_sheet_table(
    path: &Path,
    sheet_name: &str,
    header_row: u32,
) -> Result<SheetTable, String> {
    let sheet = load_sheet_xml(path, sheet_name)?;
    let parsed = parse_sheet(&sheet.xml, &sheet.shared);
    let header_row = header_row.max(1);

    let header_max_col = parsed
        .iter()
        .find(|row| row.index == header_row)
        .map(|row| row.cells.iter().map(|cell| cell.col).max().unwrap_or(0))
        .unwrap_or(0);
    let content_max_col = parsed
        .iter()
        .flat_map(|row| {
            row.cells
                .iter()
                .filter(|cell| !cell.text.trim().is_empty())
                .map(|cell| cell.col)
        })
        .max()
        .unwrap_or(0);
    let max_col = header_max_col.max(content_max_col);

    let mut headers = vec![String::new(); max_col as usize];
    if let Some(header) = parsed.iter().find(|row| row.index == header_row) {
        for cell in &header.cells {
            if cell.col >= 1 && cell.col as usize <= headers.len() {
                headers[(cell.col - 1) as usize] = cell.text.trim().to_string();
            }
        }
    }
    for (index, header) in headers.iter_mut().enumerate() {
        if header.is_empty() {
            *header = index_to_col(index as u32 + 1);
        }
    }

    let mut rows: Vec<SheetRowData> = parsed
        .iter()
        .filter(|row| row.index > header_row && !row.cells.is_empty())
        .map(|row| {
            let mut cells = vec![String::new(); max_col as usize];
            for cell in &row.cells {
                if cell.col >= 1 && cell.col as usize <= cells.len() {
                    cells[(cell.col - 1) as usize] = cell.text.trim().to_string();
                }
            }
            SheetRowData {
                index: row.index,
                cells,
            }
        })
        .collect();
    rows.sort_by_key(|row| row.index);

    Ok(SheetTable {
        name: sheet.name,
        headers,
        rows,
    })
}

/// 只改动传入的列，`None` 表示保留原单元格（含样式与公式）。
pub fn update_row(
    path: &Path,
    sheet_name: &str,
    row_index: u32,
    values: &[Option<CellValue>],
) -> Result<(), String> {
    check_writable(path)?;
    let sheet = load_sheet_xml(path, sheet_name)?;
    let (start, end) = find_row_span(&sheet.xml, row_index)
        .ok_or_else(|| format!("工作表「{}」中找不到第 {} 行，请刷新后重试", sheet.name, row_index))?;
    let block = &sheet.xml[start..end];
    if block.contains("<f") {
        return Err("这一行含有公式，请直接在 Excel 中修改".to_string());
    }
    let new_block = rebuild_row(block, row_index, values)?;
    let mut updated = format!("{}{}{}", &sheet.xml[..start], new_block, &sheet.xml[end..]);
    let parsed = parse_sheet(&updated, &sheet.shared);
    let (max_row, max_col) = max_row_col(&parsed);
    if max_row > 0 && max_col > 0 {
        updated = update_dimension(&updated, max_row, max_col);
    }
    write_archive(path, &sheet.part, &updated)
}

/// 删除一行并把它下面的行整体上移，保持 Excel 的连续行号。
pub fn delete_row(path: &Path, sheet_name: &str, row_index: u32) -> Result<(), String> {
    check_writable(path)?;
    let sheet = load_sheet_xml(path, sheet_name)?;
    if sheet.xml.contains("<f>") || sheet.xml.contains("<f ") {
        return Err("工作表含有公式，删除行会破坏公式引用，请直接在 Excel 中操作".to_string());
    }
    if sheet.xml.contains("<mergeCell") {
        return Err("工作表含有合并单元格，请直接在 Excel 中删除行".to_string());
    }
    let spans = row_spans(&sheet.xml);
    if !spans.iter().any(|(index, _, _)| *index == row_index) {
        return Err(format!(
            "工作表「{}」中找不到第 {} 行，请刷新后重试",
            sheet.name, row_index
        ));
    }

    let mut out = String::with_capacity(sheet.xml.len());
    let mut cursor = 0;
    for (index, start, end) in &spans {
        if *start < cursor {
            continue;
        }
        out.push_str(&sheet.xml[cursor..*start]);
        if *index == row_index {
            // 跳过被删除的行
        } else if *index > row_index {
            out.push_str(&renumber_row_block(&sheet.xml[*start..*end], index - 1));
        } else {
            out.push_str(&sheet.xml[*start..*end]);
        }
        cursor = *end;
    }
    out.push_str(&sheet.xml[cursor..]);

    let parsed = parse_sheet(&out, &sheet.shared);
    let (max_row, max_col) = max_row_col(&parsed);
    let updated = if max_row > 0 && max_col > 0 {
        update_dimension(&out, max_row, max_col)
    } else {
        out
    };
    write_archive(path, &sheet.part, &updated)
}

pub struct ColumnSpec {
    /// 原 Excel 列号；0 表示新增列。
    pub source: u32,
    pub label: String,
    pub width: u32,
}

/// 按给定的列顺序重建所有行：支持改名、拖拽换位、删除和新增列，
/// 同时把列宽写回工作表的 `<cols>`。
pub fn save_columns(
    path: &Path,
    sheet_name: &str,
    header_row: u32,
    columns: &[ColumnSpec],
) -> Result<(), String> {
    if columns.is_empty() {
        return Err("至少保留一列".to_string());
    }
    check_writable(path)?;
    let sheet = load_sheet_xml(path, sheet_name)?;
    if sheet.xml.contains("<f>") || sheet.xml.contains("<f ") {
        return Err("工作表含有公式，调整列会破坏公式引用，请直接在 Excel 中操作".to_string());
    }
    if sheet.xml.contains("<mergeCell") {
        return Err("工作表含有合并单元格，请直接在 Excel 中调整列".to_string());
    }
    let header_row = header_row.max(1);
    let spans = row_spans(&sheet.xml);
    if spans.is_empty() {
        return Err("工作表里没有数据".to_string());
    }
    let parsed = parse_sheet(&sheet.xml, &sheet.shared);
    let header_style = parsed
        .iter()
        .find(|row| row.index == header_row)
        .and_then(|row| row.cells.iter().find_map(|cell| cell.style.clone()));
    let mut max_source = 0u32;
    let mut seen_sources = HashMap::new();
    for row in &parsed {
        for cell in &row.cells {
            max_source = max_source.max(cell.col);
        }
    }
    for column in columns {
        if column.source > 0 {
            if column.source > max_source {
                return Err("列不存在，请刷新后重试".to_string());
            }
            if seen_sources.insert(column.source, true).is_some() {
                return Err("列设置有重复，请刷新后重试".to_string());
            }
        }
    }

    let mut out = String::with_capacity(sheet.xml.len());
    let mut cursor = 0;
    let mut max_row = 0;
    for (index, start, end) in &spans {
        if *start < cursor {
            continue;
        }
        out.push_str(&sheet.xml[cursor..*start]);
        let block = &sheet.xml[*start..*end];
        let rebuilt = if *index == header_row {
            rebuild_columns_row(block, columns, true, header_style.as_deref())?
        } else if *index > header_row {
            rebuild_columns_row(block, columns, false, header_style.as_deref())?
        } else {
            block.to_string()
        };
        out.push_str(&rebuilt);
        max_row = max_row.max(*index);
        cursor = *end;
    }
    out.push_str(&sheet.xml[cursor..]);

    let with_cols = replace_column_widths(&out, columns);
    let updated = if max_row > 0 {
        update_dimension(&with_cols, max_row, columns.len() as u32)
    } else {
        with_cols
    };
    write_archive(path, &sheet.part, &updated)
}

fn rebuild_columns_row(
    block: &str,
    columns: &[ColumnSpec],
    is_header: bool,
    header_style: Option<&str>,
) -> Result<String, String> {
    let tag_offset = block.find('>').ok_or_else(|| "行结构异常".to_string())?;
    let open_tag = &block[..=tag_offset];
    let self_closing = open_tag.trim_end().ends_with("/>");
    let row_index = row_index_from_tag(open_tag).unwrap_or(0);
    let cells = collect_row_cells(block, tag_offset + 1);

    let mut out_cells = String::new();
    for (position, column) in columns.iter().enumerate() {
        let dest = position as u32 + 1;
        let source_cell = if column.source > 0 {
            cells.iter().find(|cell| cell.col == column.source)
        } else {
            None
        };
        if is_header {
            let style = source_cell
                .and_then(|cell| cell.style.as_deref())
                .or(header_style);
            out_cells.push_str(&cell_xml(
                dest,
                row_index,
                &CellValue::Text(column.label.clone()),
                style,
            ));
        } else if let Some(cell) = source_cell {
            out_cells.push_str(&shift_cell_column(&block[cell.start..cell.end], dest));
        }
    }

    let base_tag = if self_closing {
        format!("{}>", open_tag[..open_tag.len() - 2].trim_end())
    } else {
        open_tag.to_string()
    };
    Ok(format!("{}{}</row>", base_tag, out_cells))
}

fn replace_column_widths(xml: &str, columns: &[ColumnSpec]) -> String {
    let without_cols = remove_cols_block(xml);
    let Some(position) = without_cols.find("<sheetData") else {
        return without_cols;
    };
    let mut block = String::from("<cols>");
    for (index, column) in columns.iter().enumerate() {
        let width = column.width.clamp(6, 80);
        block.push_str(&format!(
            "<col min=\"{}\" max=\"{}\" width=\"{}\" customWidth=\"1\"/>",
            index + 1,
            index + 1,
            width
        ));
    }
    block.push_str("</cols>");
    let mut out = String::with_capacity(without_cols.len() + block.len());
    out.push_str(&without_cols[..position]);
    out.push_str(&block);
    out.push_str(&without_cols[position..]);
    out
}

fn remove_cols_block(xml: &str) -> String {
    let Some(start) = xml.find("<cols") else {
        return xml.to_string();
    };
    let Some(tag_offset) = xml[start..].find('>') else {
        return xml.to_string();
    };
    let tag_end = start + tag_offset + 1;
    if xml[start..tag_end].trim_end().ends_with("/>") {
        let mut out = String::with_capacity(xml.len());
        out.push_str(&xml[..start]);
        out.push_str(&xml[tag_end..]);
        return out;
    }
    let Some(close_offset) = xml[tag_end..].find("</cols>") else {
        return xml.to_string();
    };
    let close_end = tag_end + close_offset + "</cols>".len();
    let mut out = String::with_capacity(xml.len());
    out.push_str(&xml[..start]);
    out.push_str(&xml[close_end..]);
    out
}

/// 删除一列：从每一行移除该列，后面的列整体左移。
pub fn remove_column(path: &Path, sheet_name: &str, column: u32) -> Result<(), String> {
    if column == 0 {
        return Err("列号无效".to_string());
    }
    check_writable(path)?;
    let sheet = load_sheet_xml(path, sheet_name)?;
    if sheet.xml.contains("<f>") || sheet.xml.contains("<f ") {
        return Err("工作表含有公式，删除列会破坏公式引用，请直接在 Excel 中操作".to_string());
    }
    if sheet.xml.contains("<mergeCell") {
        return Err("工作表含有合并单元格，请直接在 Excel 中删除列".to_string());
    }
    let spans = row_spans(&sheet.xml);
    if spans.is_empty() {
        return Err("工作表里没有数据".to_string());
    }

    let mut out = String::with_capacity(sheet.xml.len());
    let mut cursor = 0;
    let mut max_col = 0u32;
    let mut max_row = 0u32;
    for (index, start, end) in &spans {
        if *start < cursor {
            continue;
        }
        out.push_str(&sheet.xml[cursor..*start]);
        let (block, block_max_col) = remove_column_from_row(&sheet.xml[*start..*end], column)?;
        out.push_str(&block);
        max_col = max_col.max(block_max_col);
        max_row = max_row.max(*index);
        cursor = *end;
    }
    out.push_str(&sheet.xml[cursor..]);

    let shifted = adjust_column_definitions(&out, column);
    let updated = if max_row > 0 && max_col > 0 {
        update_dimension(&shifted, max_row, max_col)
    } else {
        shifted
    };
    write_archive(path, &sheet.part, &updated)
}

/// 同步调整 `<cols>` 里的列宽定义，避免删列后宽度错位。
fn adjust_column_definitions(xml: &str, removed: u32) -> String {
    let Some(start) = xml.find("<cols") else {
        return xml.to_string();
    };
    let Some(start_end_offset) = xml[start..].find('>') else {
        return xml.to_string();
    };
    let start_end = start + start_end_offset + 1;
    let open_tag = &xml[start..start_end];
    if open_tag.trim_end().ends_with("/>") {
        return xml.to_string();
    }
    let Some(close_offset) = xml[start_end..].find("</cols>") else {
        return xml.to_string();
    };
    let close = start_end + close_offset;
    let close_end = close + "</cols>".len();

    let inner = &xml[start_end..close];
    let mut rebuilt = String::new();
    let mut cursor = 0;
    while let Some(relative) = inner[cursor..].find("<col") {
        let col_start = cursor + relative;
        let next = inner.as_bytes().get(col_start + 4).copied();
        if !matches!(next, Some(b' ') | Some(b'>') | Some(b'\t') | Some(b'\n') | Some(b'\r')) {
            rebuilt.push_str(&inner[cursor..col_start + 4]);
            cursor = col_start + 4;
            continue;
        }
        let Some(tag_offset) = inner[col_start..].find('>') else {
            break;
        };
        let tag_end = col_start + tag_offset + 1;
        rebuilt.push_str(&inner[cursor..col_start]);
        let tag = &inner[col_start..tag_end];
        if let Some(adjusted) = adjust_col_tag(tag, removed) {
            rebuilt.push_str(&adjusted);
        }
        cursor = tag_end;
    }
    rebuilt.push_str(&inner[cursor..]);

    if rebuilt.trim().is_empty() {
        let mut out = String::with_capacity(xml.len());
        out.push_str(&xml[..start]);
        out.push_str(&xml[close_end..]);
        out
    } else {
        format!("{}{}{}</cols>{}", &xml[..start], open_tag, rebuilt, &xml[close_end..])
    }
}

fn adjust_col_tag(tag: &str, removed: u32) -> Option<String> {
    let min = attr_of(tag, "min")?.parse::<u32>().ok()?;
    let max = attr_of(tag, "max")?.parse::<u32>().ok()?;
    if max < removed {
        return Some(tag.to_string());
    }
    if min > removed {
        let tag = replace_attr(tag, "min", &(min - 1).to_string());
        return Some(replace_attr(&tag, "max", &(max - 1).to_string()));
    }
    if min >= max {
        return None;
    }
    Some(replace_attr(tag, "max", &(max - 1).to_string()))
}

fn replace_attr(tag: &str, key: &str, value: &str) -> String {
    let Some(current) = attr_of(tag, key) else {
        return tag.to_string();
    };
    tag.replacen(
        &format!("{}=\"{}\"", key, current),
        &format!("{}=\"{}\"", key, value),
        1,
    )
}

fn remove_column_from_row(block: &str, column: u32) -> Result<(String, u32), String> {
    let tag_offset = block
        .find('>')
        .ok_or_else(|| "行结构异常".to_string())?;
    let open_tag = &block[..=tag_offset];
    let self_closing = open_tag.trim_end().ends_with("/>");
    let cells = collect_row_cells(block, tag_offset + 1);

    let mut out_cells = String::new();
    let mut max_col = 0u32;
    for cell in &cells {
        if cell.col == column {
            continue;
        }
        if cell.col > column {
            let new_col = cell.col - 1;
            out_cells.push_str(&shift_cell_column(&block[cell.start..cell.end], new_col));
            max_col = max_col.max(new_col);
        } else {
            out_cells.push_str(&block[cell.start..cell.end]);
            max_col = max_col.max(cell.col);
        }
    }

    let base_tag = if self_closing {
        format!("{}>", open_tag[..open_tag.len() - 2].trim_end())
    } else {
        open_tag.to_string()
    };
    Ok((format!("{}{}</row>", base_tag, out_cells), max_col))
}

fn shift_cell_column(cell: &str, new_col: u32) -> String {
    let Some(reference) = attr_of(cell, "r") else {
        return cell.to_string();
    };
    let row_part: String = reference
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect();
    let replacement = format!("r=\"{}{}\"", index_to_col(new_col), row_part);
    cell.replacen(&format!("r=\"{}\"", reference), &replacement, 1)
}

/// 按前端给出的新顺序重排数据行。
/// `order` 里是每一行调整前的 Excel 行号，按新的从上到下顺序排列；
/// 行号本身不变，只把行内容搬到对应的物理行上，空行位置保持不动。
pub fn reorder_rows(
    path: &Path,
    sheet_name: &str,
    header_row: u32,
    order: &[u32],
) -> Result<(), String> {
    check_writable(path)?;
    let sheet = load_sheet_xml(path, sheet_name)?;
    if sheet.xml.contains("<f>") || sheet.xml.contains("<f ") {
        return Err("工作表含有公式，调整行顺序会破坏公式引用，请直接在 Excel 中操作".to_string());
    }
    if sheet.xml.contains("<mergeCell") {
        return Err("工作表含有合并单元格，请直接在 Excel 中调整行顺序".to_string());
    }
    let header_row = header_row.max(1);
    let spans = row_spans(&sheet.xml);
    let data_spans: Vec<(u32, usize, usize)> = spans
        .iter()
        .filter(|(index, _, _)| *index > header_row)
        .cloned()
        .collect();
    if data_spans.is_empty() {
        return Err("表格里没有可调整的数据行".to_string());
    }

    let mut slots: Vec<u32> = data_spans.iter().map(|(index, _, _)| *index).collect();
    slots.sort_unstable();
    let mut provided = order.to_vec();
    provided.sort_unstable();
    if slots != provided {
        return Err("表格内容已变化，请刷新后再调整顺序".to_string());
    }

    let mut blocks: HashMap<u32, &str> = HashMap::new();
    for (index, start, end) in &data_spans {
        blocks.insert(*index, &sheet.xml[*start..*end]);
    }
    let mut replacements: HashMap<u32, String> = HashMap::new();
    for (position, slot) in slots.iter().enumerate() {
        let source = order[position];
        let block = blocks
            .get(&source)
            .ok_or_else(|| "表格内容已变化，请刷新后再调整顺序".to_string())?;
        replacements.insert(*slot, renumber_row_block(block, *slot));
    }

    let mut out = String::with_capacity(sheet.xml.len());
    let mut cursor = 0;
    for (index, start, end) in &spans {
        if *start < cursor {
            continue;
        }
        out.push_str(&sheet.xml[cursor..*start]);
        if let Some(replacement) = replacements.get(index) {
            out.push_str(replacement);
        } else {
            out.push_str(&sheet.xml[*start..*end]);
        }
        cursor = *end;
    }
    out.push_str(&sheet.xml[cursor..]);
    write_archive(path, &sheet.part, &out)
}

/// 在表格末尾追加一行，列按位置对齐。
pub fn append_row_aligned(
    path: &Path,
    sheet_name: &str,
    header_row: u32,
    values: &[CellValue],
) -> Result<u32, String> {
    check_writable(path)?;
    let sheet = load_sheet_xml(path, sheet_name)?;
    let parsed = parse_sheet(&sheet.xml, &sheet.shared);
    let (max_row, max_col) = max_row_col(&parsed);
    let row_index = max_row.max(header_row.max(1)) + 1;

    let mut cells = String::new();
    let mut new_max_col = max_col;
    for (index, value) in values.iter().enumerate() {
        let col = index as u32 + 1;
        new_max_col = new_max_col.max(col);
        cells.push_str(&cell_xml(col, row_index, value, None));
    }
    let row_xml = format!("<row r=\"{}\">{}</row>", row_index, cells);
    let insert_at = sheet
        .xml
        .find("</sheetData>")
        .ok_or("工作表结构异常，找不到 sheetData")?;
    let updated = format!(
        "{}{}{}",
        &sheet.xml[..insert_at],
        row_xml,
        &sheet.xml[insert_at..]
    );
    let updated = update_dimension(&updated, row_index, new_max_col);
    write_archive(path, &sheet.part, &updated)?;
    Ok(row_index)
}

fn find_row_span(xml: &str, target: u32) -> Option<(usize, usize)> {
    let (start, tag_end, self_closing) = find_row_tag(xml, target)?;
    if self_closing {
        Some((start, tag_end))
    } else {
        let close = xml[tag_end..].find("</row>")?;
        Some((start, tag_end + close + "</row>".len()))
    }
}

fn row_spans(xml: &str) -> Vec<(u32, usize, usize)> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = xml[cursor..].find("<row") {
        let start = cursor + relative;
        let next = xml.as_bytes().get(start + 4).copied();
        if !matches!(next, Some(b' ') | Some(b'>') | Some(b'\t') | Some(b'\n') | Some(b'\r')) {
            cursor = start + 4;
            continue;
        }
        let Some(tag_offset) = xml[start..].find('>') else {
            break;
        };
        let tag_end = start + tag_offset;
        let self_closing = xml[start..=tag_end].trim_end().ends_with("/>");
        let end = if self_closing {
            tag_end + 1
        } else {
            match xml[tag_end..].find("</row>") {
                Some(offset) => tag_end + offset + "</row>".len(),
                None => break,
            }
        };
        let index = row_index_from_tag(&xml[start..=tag_end]).unwrap_or(0);
        out.push((index, start, end));
        cursor = end;
    }
    out
}

fn rebuild_row(
    block: &str,
    row_index: u32,
    values: &[Option<CellValue>],
) -> Result<String, String> {
    let (_, tag_end, self_closing) = find_row_tag(block, row_index)
        .ok_or_else(|| format!("第 {} 行结构异常", row_index))?;
    let open_tag = &block[..tag_end];
    let base_tag = if self_closing {
        format!("{}>", open_tag[..open_tag.len() - 2].trim_end())
    } else {
        open_tag.to_string()
    };

    let cells = collect_row_cells(block, tag_end);
    let mut max_col = values.len() as u32;
    for cell in &cells {
        max_col = max_col.max(cell.col);
    }
    // 新增列时没有原单元格，沿用本行已有单元格的样式，让新表头和其它表头一致。
    let fallback_style = cells.iter().find_map(|cell| cell.style.clone());
    let mut out_cells = String::new();
    for col in 1..=max_col {
        let existing = cells.iter().find(|cell| cell.col == col);
        match values.get(col as usize - 1) {
            Some(Some(value)) => {
                if value.is_empty() {
                    // 清空内容时保留原单元格格式，方便以后再填。
                    if let Some(cell) = existing {
                        out_cells.push_str(&empty_cell_xml(col, row_index, cell.style.as_deref()));
                    }
                    continue;
                }
                out_cells.push_str(&cell_xml(
                    col,
                    row_index,
                    value,
                    existing
                        .and_then(|cell| cell.style.as_deref())
                        .or(fallback_style.as_deref()),
                ));
            }
            _ => {
                if let Some(cell) = existing {
                    out_cells.push_str(&block[cell.start..cell.end]);
                }
            }
        }
    }
    Ok(format!("{}{}</row>", base_tag, out_cells))
}

fn collect_row_cells(block: &str, from: usize) -> Vec<RowCellSpan> {
    let mut out = Vec::new();
    let mut cursor = from;
    while let Some(relative) = block[cursor..].find("<c") {
        let start = cursor + relative;
        let next = block.as_bytes().get(start + 2).copied();
        if !matches!(next, Some(b' ') | Some(b'>') | Some(b'\t') | Some(b'\n') | Some(b'\r')) {
            cursor = start + 2;
            continue;
        }
        let Some(tag_offset) = block[start..].find('>') else {
            break;
        };
        let tag_end = start + tag_offset;
        let self_closing = block[start..=tag_end].trim_end().ends_with("/>");
        let end = if self_closing {
            tag_end + 1
        } else {
            match block[tag_end..].find("</c>") {
                Some(offset) => tag_end + offset + "</c>".len(),
                None => break,
            }
        };
        let tag = &block[start..=tag_end];
        let col = attr_of(tag, "r")
            .map(|reference| col_to_index(&reference))
            .unwrap_or(0);
        out.push(RowCellSpan {
            col,
            start,
            end,
            style: attr_of(tag, "s"),
        });
        cursor = end;
    }
    out
}

fn attr_of(tag: &str, key: &str) -> Option<String> {
    let needle = format!("{}=\"", key);
    let bytes = tag.as_bytes();
    let mut search = 0;
    while let Some(relative) = tag[search..].find(&needle) {
        let position = search + relative;
        let boundary = position == 0 || bytes[position - 1].is_ascii_whitespace();
        if boundary {
            let value_start = position + needle.len();
            let value_end = tag[value_start..].find('"')? + value_start;
            return Some(tag[value_start..value_end].to_string());
        }
        search = position + needle.len();
    }
    None
}

fn renumber_row_block(block: &str, new_row: u32) -> String {
    let mut out = String::with_capacity(block.len());
    let mut cursor = 0;
    if let Some(relative) = block.find("<row") {
        let start = relative;
        if let Some(offset) = block[start..].find('>') {
            let tag_end = start + offset;
            out.push_str(&block[..start]);
            out.push_str(&rewrite_ref(&block[start..=tag_end], new_row));
            cursor = tag_end + 1;
        }
    }
    while let Some(relative) = block[cursor..].find("<c") {
        let start = cursor + relative;
        let next = block.as_bytes().get(start + 2).copied();
        if !matches!(next, Some(b' ') | Some(b'>') | Some(b'\t') | Some(b'\n') | Some(b'\r')) {
            out.push_str(&block[cursor..start + 2]);
            cursor = start + 2;
            continue;
        }
        let Some(offset) = block[start..].find('>') else {
            break;
        };
        let tag_end = start + offset;
        out.push_str(&block[cursor..start]);
        out.push_str(&rewrite_ref(&block[start..=tag_end], new_row));
        cursor = tag_end + 1;
    }
    out.push_str(&block[cursor..]);
    out
}

fn rewrite_ref(tag: &str, new_row: u32) -> String {
    let Some(reference) = attr_of(tag, "r") else {
        return tag.to_string();
    };
    let letters: String = reference
        .chars()
        .take_while(|ch| ch.is_ascii_alphabetic())
        .collect();
    let replacement = format!("r=\"{}{}\"", letters, new_row);
    tag.replacen(&format!("r=\"{}\"", reference), &replacement, 1)
}

/// 把配置里的列名同步到工作簿的指定工作表表头行，保留其它数据。
pub fn sync_headers(
    path: &Path,
    sheet_name: &str,
    header_row: u32,
    headers: &[String],
) -> Result<(), String> {
    if headers.is_empty() {
        return Ok(());
    }
    if !path.exists() {
        create_workbook(path, sheet_name, headers, &[])?;
        return Ok(());
    }
    if excel_lock_exists(path) {
        return Err("目标文件正在 Excel 中打开，请先关闭后再保存配置".to_string());
    }
    let file = fs::File::open(path).map_err(|e| format!("无法打开目标文件：{}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| "目标文件不是有效的 xlsx 工作簿".to_string())?;
    let workbook_xml =
        read_entry(&mut archive, "xl/workbook.xml").ok_or("工作簿缺少 xl/workbook.xml")?;
    let rels_xml =
        read_entry(&mut archive, "xl/_rels/workbook.xml.rels").ok_or("工作簿缺少关系定义")?;
    let sheets = parse_sheets(&workbook_xml);
    let rels = parse_rels(&rels_xml);
    let wanted = sheet_name.trim();
    let matched = sheets
        .iter()
        .find(|(name, _)| name == wanted)
        .or_else(|| if sheets.len() == 1 { sheets.first() } else { None })
        .cloned()
        .ok_or_else(|| format!("工作簿中没有工作表「{}」", wanted))?;
    let part = rel_target(&matched.1, &rels)?;
    let sheet_xml = read_entry(&mut archive, &part)
        .ok_or_else(|| format!("工作表「{}」的数据缺失", matched.0))?;
    let shared = read_entry(&mut archive, "xl/sharedStrings.xml")
        .map(|xml| parse_shared_strings(&xml))
        .unwrap_or_default();
    let parsed = parse_sheet(&sheet_xml, &shared);
    let max_row = parsed.iter().map(|row| row.index).max().unwrap_or(0);
    let max_col = parsed
        .iter()
        .flat_map(|row| row.cells.iter().map(|cell| cell.col))
        .max()
        .unwrap_or(0);

    let header_row = header_row.max(1);
    let mut cells = String::new();
    let mut new_max_col = max_col;
    for (index, header) in headers.iter().enumerate() {
        let col = index as u32 + 1;
        new_max_col = new_max_col.max(col);
        cells.push_str(&cell_xml(
            col,
            header_row,
            &CellValue::Text(header.clone()),
            None,
        ));
    }
    let row_xml = format!("<row r=\"{}\">{}</row>", header_row, cells);

    let updated = if let Some((start, end, self_closing)) = find_row_tag(&sheet_xml, header_row) {
        let row_end = if self_closing {
            end
        } else {
            sheet_xml[end..]
                .find("</row>")
                .map(|offset| end + offset + "</row>".len())
                .ok_or_else(|| format!("第 {} 行结构异常", header_row))?
        };
        format!("{}{}{}", &sheet_xml[..start], row_xml, &sheet_xml[row_end..])
    } else {
        let insert_at = first_row_start_after(&sheet_xml, header_row)
            .or_else(|| sheet_xml.find("</sheetData>"))
            .ok_or("工作表结构异常")?;
        format!(
            "{}{}{}",
            &sheet_xml[..insert_at],
            row_xml,
            &sheet_xml[insert_at..]
        )
    };
    let updated = update_dimension(&updated, max_row.max(header_row), new_max_col);
    write_archive(path, &part, &updated)
}

fn first_row_start_after(xml: &str, row: u32) -> Option<usize> {
    let bytes = xml.as_bytes();
    let mut cursor = 0;
    while let Some(offset) = xml[cursor..].find("<row ") {
        let start = cursor + offset;
        let tag_end = xml[start..].find('>')? + start;
        let tag = &xml[start + 5..tag_end];
        if let Some(index) = tag
            .split_whitespace()
            .find_map(|part| part.strip_prefix("r=\""))
            .and_then(|rest| rest.trim_end_matches('"').parse::<u32>().ok())
        {
            if index > row {
                return Some(start);
            }
        }
        cursor = tag_end + 1;
        if cursor >= bytes.len() {
            break;
        }
    }
    None
}

/// 没有列定义时：把每条内容按顺序写进第一列。
pub fn append_plain_column(
    path: &Path,
    sheet_name: &str,
    values: &[String],
) -> Result<AppendReport, String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(format!("目标目录不存在：{}", parent.display()));
        }
    }
    if !path.exists() {
        let rows: Vec<Vec<CellValue>> = values
            .iter()
            .map(|value| vec![CellValue::Text(value.clone())])
            .collect();
        create_workbook(path, sheet_name, &[], &rows)?;
        return Ok(AppendReport {
            written: values.len(),
            added_columns: Vec::new(),
            sheet: if sheet_name.trim().is_empty() {
                "Sheet1".to_string()
            } else {
                sheet_name.trim().to_string()
            },
        });
    }
    if excel_lock_exists(path) {
        return Err("目标文件正在 Excel 中打开，请先关闭后再导出".to_string());
    }
    let file = fs::File::open(path).map_err(|e| format!("无法打开目标文件：{}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| "目标文件不是有效的 xlsx 工作簿".to_string())?;
    let workbook_xml =
        read_entry(&mut archive, "xl/workbook.xml").ok_or("工作簿缺少 xl/workbook.xml")?;
    let rels_xml =
        read_entry(&mut archive, "xl/_rels/workbook.xml.rels").ok_or("工作簿缺少关系定义")?;
    let sheets = parse_sheets(&workbook_xml);
    let rels = parse_rels(&rels_xml);
    let wanted = sheet_name.trim();
    let matched = sheets
        .iter()
        .find(|(name, _)| name == wanted)
        .or_else(|| if sheets.len() == 1 { sheets.first() } else { None })
        .cloned()
        .ok_or_else(|| format!("工作簿中没有工作表「{}」", wanted))?;
    let part = rel_target(&matched.1, &rels)?;
    let sheet_xml = read_entry(&mut archive, &part)
        .ok_or_else(|| format!("工作表「{}」的数据缺失", matched.0))?;
    let shared = read_entry(&mut archive, "xl/sharedStrings.xml")
        .map(|xml| parse_shared_strings(&xml))
        .unwrap_or_default();
    let parsed = parse_sheet(&sheet_xml, &shared);
    let mut max_row = parsed.iter().map(|row| row.index).max().unwrap_or(0);
    let max_col = parsed
        .iter()
        .flat_map(|row| row.cells.iter().map(|cell| cell.col))
        .max()
        .unwrap_or(0);
    let mut rows_xml = String::new();
    for value in values {
        max_row += 1;
        rows_xml.push_str(&format!(
            "<row r=\"{}\">{}</row>",
            max_row,
            cell_xml(1, max_row, &CellValue::Text(value.clone()), None)
        ));
    }
    let insert_at = sheet_xml.find("</sheetData>").ok_or("工作表结构异常")?;
    let updated = format!(
        "{}{}{}",
        &sheet_xml[..insert_at],
        rows_xml,
        &sheet_xml[insert_at..]
    );
    let updated = update_dimension(&updated, max_row, max_col);
    write_archive(path, &part, &updated)?;
    Ok(AppendReport {
        written: values.len(),
        added_columns: Vec::new(),
        sheet: matched.0,
    })
}

fn excel_lock_exists(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    path.with_file_name(format!("~${}", name)).exists()
}

fn write_archive(path: &Path, part: &str, new_xml: &str) -> Result<(), String> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("ledger");
    let temp = path.with_file_name(format!(".{}.{}.tmp", stem, std::process::id()));

    {
        let file = fs::File::open(path).map_err(|e| format!("无法打开目标文件：{}", e))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|_| "目标文件不是有效的 xlsx 工作簿".to_string())?;
        let out = fs::File::create(&temp).map_err(|e| format!("无法写入临时文件：{}", e))?;
        let mut writer = zip::ZipWriter::new(out);

        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|e| format!("读取工作簿条目失败：{}", e))?;
            let name = entry.name().unwrap_or_default().to_string();
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(entry.compression())
                .unix_permissions(entry.unix_mode().unwrap_or(0o644));

            if entry.is_dir() {
                writer
                    .add_directory(name.trim_end_matches('/'), options)
                    .map_err(|e| format!("写入工作簿失败：{}", e))?;
                continue;
            }

            writer
                .start_file(name.clone(), options)
                .map_err(|e| format!("写入工作簿失败：{}", e))?;
            if name == part {
                writer
                    .write_all(new_xml.as_bytes())
                    .map_err(|e| format!("写入工作表失败：{}", e))?;
            } else {
                let mut buffer = Vec::with_capacity(entry.size() as usize);
                entry
                    .read_to_end(&mut buffer)
                    .map_err(|e| format!("读取工作簿条目失败：{}", e))?;
                writer
                    .write_all(&buffer)
                    .map_err(|e| format!("写入工作簿失败：{}", e))?;
            }
        }
        writer
            .finish()
            .map_err(|e| format!("写入工作簿失败：{}", e))?;
    }

    fs::rename(&temp, path).map_err(|e| {
        let _ = fs::remove_file(&temp);
        format!("替换目标文件失败（可能被占用或没有权限）：{}", e)
    })
}

fn read_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Option<String> {
    let mut entry = archive.by_name(name).ok()?;
    let mut buffer = Vec::new();
    entry.read_to_end(&mut buffer).ok()?;
    Some(String::from_utf8_lossy(&buffer).to_string())
}

fn attr(element: &BytesStart, key: &str) -> Option<String> {
    element
        .attributes()
        .with_checks(false)
        .flatten()
        .find(|attribute| attribute.key.as_ref() == key)
        .and_then(|attribute| attribute.unescape_value().ok())
        .map(|value| value.to_string())
}

fn parse_sheets(xml: &str) -> Vec<(String, String)> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                if element.name().as_ref() == "sheet" {
                    let name = attr(&element, "name").unwrap_or_default();
                    let rid = attr(&element, "r:id").unwrap_or_default();
                    if !name.is_empty() && !rid.is_empty() {
                        out.push((name, rid));
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    out
}

fn parse_rels(xml: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(xml);
    let mut out = HashMap::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                if element.name().as_ref() == "Relationship" {
                    let id = attr(&element, "Id").unwrap_or_default();
                    let target = attr(&element, "Target").unwrap_or_default();
                    if !id.is_empty() && !target.is_empty() {
                        out.insert(id, target);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    out
}

fn rel_target(rid: &str, rels: &HashMap<String, String>) -> Result<String, String> {
    let target = rels
        .get(rid)
        .ok_or_else(|| "工作簿关系定义缺失".to_string())?;
    if let Some(stripped) = target.strip_prefix('/') {
        Ok(stripped.to_string())
    } else {
        Ok(format!("xl/{}", target.trim_start_matches("./")))
    }
}

fn parse_shared_strings(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut in_item = false;
    let mut buffer = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => match element.name().as_ref() {
                "si" => {
                    in_item = true;
                    buffer.clear();
                }
                "t" if in_item => {}
                _ => {}
            },
            Ok(Event::Text(text)) => {
                if in_item {
                    let raw = text.xml10_content();
                    let value = quick_xml::escape::unescape(raw.as_ref())
                        .map(|value| value.to_string())
                        .unwrap_or_else(|_| raw.to_string());
                    buffer.push_str(&value);
                }
            }
            Ok(Event::End(element)) => {
                if element.name().as_ref() == "si" {
                    out.push(buffer.clone());
                    in_item = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    out
}

fn parse_sheet(xml: &str, shared: &[String]) -> Vec<ParsedRow> {
    let mut reader = Reader::from_str(xml);
    let mut rows = Vec::new();
    let mut current_row: Option<ParsedRow> = None;
    let mut current_cell: Option<(u32, Option<String>, String)> = None;
    let mut cell_type = String::new();
    let mut in_value = false;
    let mut in_inline = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => match element.name().as_ref() {
                "row" => {
                    let index = attr(&element, "r")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0);
                    current_row = Some(ParsedRow {
                        index,
                        cells: Vec::new(),
                    });
                }
                "c" => {
                    let reference = attr(&element, "r").unwrap_or_default();
                    current_cell = Some((
                        col_to_index(&reference),
                        attr(&element, "s"),
                        String::new(),
                    ));
                    cell_type = attr(&element, "t").unwrap_or_default();
                }
                "v" => in_value = true,
                "t" if current_cell.is_some() => in_inline = true,
                _ => {}
            },
            Ok(Event::Text(text)) => {
                if in_value || in_inline {
                    let raw = text.xml10_content();
                    let value = quick_xml::escape::unescape(raw.as_ref())
                        .map(|value| value.to_string())
                        .unwrap_or_else(|_| raw.to_string());
                    if let Some((_, _, buffer)) = current_cell.as_mut() {
                        buffer.push_str(&value);
                    }
                }
            }
            Ok(Event::End(element)) => match element.name().as_ref() {
                "v" => in_value = false,
                "t" => in_inline = false,
                "c" => {
                    if let (Some(row), Some((col, style, raw))) =
                        (current_row.as_mut(), current_cell.take())
                    {
                        let text = if cell_type == "s" {
                            raw.parse::<usize>()
                                .ok()
                                .and_then(|index| shared.get(index).cloned())
                                .unwrap_or_default()
                        } else {
                            raw
                        };
                        row.cells.push(ParsedCell { col, text, style });
                    }
                }
                "row" => {
                    if let Some(row) = current_row.take() {
                        rows.push(row);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    rows
}

fn find_row_tag(xml: &str, target: u32) -> Option<(usize, usize, bool)> {
    let mut position = 0;
    while let Some(relative) = xml[position..].find("<row") {
        let start = position + relative;
        let rest = &xml[start + 4..];
        let end_relative = rest.find('>')?;
        let end = start + 4 + end_relative;
        let tag = &xml[start..=end];
        if let Some(index) = row_index_from_tag(tag) {
            if index == target {
                let self_closing = tag.trim_end().ends_with("/>");
                return Some((start, end + 1, self_closing));
            }
        }
        position = end + 1;
    }
    None
}

fn row_index_from_tag(tag: &str) -> Option<u32> {
    let start = tag.find("r=\"")?;
    let rest = &tag[start + 3..];
    let end = rest.find('"')?;
    rest[..end].trim().parse().ok()
}

fn col_to_index(reference: &str) -> u32 {
    let mut value = 0u32;
    for byte in reference.bytes() {
        if byte.is_ascii_alphabetic() {
            value = value * 26 + (byte.to_ascii_uppercase() - b'A' + 1) as u32;
        } else {
            break;
        }
    }
    value
}

fn index_to_col(mut index: u32) -> String {
    let mut out = String::new();
    while index > 0 {
        let remainder = ((index - 1) % 26) as u8 + b'A';
        out.insert(0, remainder as char);
        index = (index - 1) / 26;
    }
    if out.is_empty() {
        out.push('A');
    }
    out
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_sheet_rows(path: &Path) -> Vec<ParsedRow> {
        let file = fs::File::open(path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let sheet = read_entry(&mut archive, "xl/worksheets/sheet1.xml").unwrap();
        let shared = read_entry(&mut archive, "xl/sharedStrings.xml")
            .map(|xml| parse_shared_strings(&xml))
            .unwrap_or_default();
        parse_sheet(&sheet, &shared)
    }

    #[test]
    fn append_matches_headers_and_adds_missing_columns() {
        let path = std::env::temp_dir().join(format!(
            "jy-ledger-{}.xlsx",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let headers = vec!["医院".to_string(), "经销商".to_string()];
        append_to_workbook(
            &path,
            "绿通申请",
            1,
            &headers,
            &[vec![
                CellValue::Text("上海市浦东医院".into()),
                CellValue::Text("上海影骊".into()),
            ]],
        )
        .unwrap();

        // 第二次追加：多一列「数量」，应按列名补到末尾。
        let report = append_to_workbook(
            &path,
            "绿通申请",
            1,
            &[
                "医院".to_string(),
                "经销商".to_string(),
                "数量".to_string(),
            ],
            &[vec![
                CellValue::Text("浙大二院".into()),
                CellValue::Text("山东有恒".into()),
                CellValue::Number(2.0),
            ]],
        )
        .unwrap();
        assert_eq!(report.written, 1);
        assert_eq!(report.added_columns, vec!["数量".to_string()]);
        assert_eq!(report.sheet, "绿通申请");

        let rows = read_sheet_rows(&path);
        let header = rows.iter().find(|row| row.index == 1).unwrap();
        assert!(header.cells.iter().any(|cell| cell.text == "数量" && cell.col == 3));
        let last = rows.iter().find(|row| row.index == 3).unwrap();
        assert!(last.cells.iter().any(|cell| cell.text == "浙大二院" && cell.col == 1));
        assert!(last.cells.iter().any(|cell| cell.text == "2" && cell.col == 3));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn read_update_append_and_delete_rows() {
        let path = std::env::temp_dir().join(format!(
            "jy-ledger-table-{}.xlsx",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        append_to_workbook(
            &path,
            "数据",
            1,
            &["医院".to_string(), "经销商".to_string()],
            &[
                vec![
                    CellValue::Text("A院".into()),
                    CellValue::Text("甲商".into()),
                ],
                vec![
                    CellValue::Text("B院".into()),
                    CellValue::Text("乙商".into()),
                ],
            ],
        )
        .unwrap();

        let table = read_sheet_table(&path, "数据", 1).unwrap();
        assert_eq!(table.headers, vec!["医院", "经销商"]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].index, 2);
        assert_eq!(table.rows[0].cells, vec!["A院", "甲商"]);

        // 只改第一列时，第二列必须原样保留。
        update_row(
            &path,
            "数据",
            2,
            &[Some(CellValue::Text("A院新".into())), None],
        )
        .unwrap();
        let table = read_sheet_table(&path, "数据", 1).unwrap();
        assert_eq!(table.rows[0].cells, vec!["A院新", "甲商"]);

        let row = append_row_aligned(
            &path,
            "数据",
            1,
            &[CellValue::Text("C院".into()), CellValue::Number(3.0)],
        )
        .unwrap();
        assert_eq!(row, 4);

        delete_row(&path, "数据", 2).unwrap();
        let table = read_sheet_table(&path, "数据", 1).unwrap();
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].index, 2);
        assert_eq!(table.rows[0].cells, vec!["B院", "乙商"]);
        assert_eq!(table.rows[1].index, 3);
        assert_eq!(table.rows[1].cells, vec!["C院", "3"]);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn reorder_rows_follows_given_order() {
        let path = std::env::temp_dir().join(format!(
            "jy-ledger-reorder-{}.xlsx",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        append_to_workbook(
            &path,
            "数据",
            1,
            &["医院".to_string()],
            &[
                vec![CellValue::Text("A院".into())],
                vec![CellValue::Text("B院".into())],
                vec![CellValue::Text("C院".into())],
            ],
        )
        .unwrap();

        // 把第 3 行拖到第 2 行上面。
        reorder_rows(&path, "数据", 1, &[3, 2, 4]).unwrap();
        let table = read_sheet_table(&path, "数据", 1).unwrap();
        assert_eq!(
            table.rows.iter().map(|row| row.index).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
        assert_eq!(table.rows[0].cells, vec!["B院"]);
        assert_eq!(table.rows[1].cells, vec!["A院"]);
        assert_eq!(table.rows[2].cells, vec!["C院"]);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn remove_column_shifts_cells_left() {
        let path = std::env::temp_dir().join(format!(
            "jy-ledger-remove-col-{}.xlsx",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        append_to_workbook(
            &path,
            "数据",
            1,
            &["A".to_string(), "B".to_string(), "C".to_string()],
            &[
                vec![
                    CellValue::Text("a1".into()),
                    CellValue::Text("b1".into()),
                    CellValue::Text("c1".into()),
                ],
                vec![
                    CellValue::Text("a2".into()),
                    CellValue::Text("b2".into()),
                    CellValue::Text("c2".into()),
                ],
            ],
        )
        .unwrap();

        remove_column(&path, "数据", 2).unwrap();
        let table = read_sheet_table(&path, "数据", 1).unwrap();
        assert_eq!(table.headers, vec!["A", "C"]);
        assert_eq!(table.rows[0].cells, vec!["a1", "c1"]);
        assert_eq!(table.rows[1].cells, vec!["a2", "c2"]);

        remove_column(&path, "数据", 1).unwrap();
        let table = read_sheet_table(&path, "数据", 1).unwrap();
        assert_eq!(table.headers, vec!["C"]);
        assert_eq!(table.rows[0].cells, vec!["c1"]);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn save_columns_renames_reorders_and_adds() {
        let path = std::env::temp_dir().join(format!(
            "jy-ledger-save-cols-{}.xlsx",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        append_to_workbook(
            &path,
            "数据",
            1,
            &["A".to_string(), "B".to_string(), "C".to_string()],
            &[
                vec![
                    CellValue::Text("a1".into()),
                    CellValue::Text("b1".into()),
                    CellValue::Text("c1".into()),
                ],
                vec![
                    CellValue::Text("a2".into()),
                    CellValue::Text("b2".into()),
                    CellValue::Text("c2".into()),
                ],
            ],
        )
        .unwrap();

        // B 提到第一列并改名，新增 D，A 放到第三列，C 删除。
        save_columns(
            &path,
            "数据",
            1,
            &[
                ColumnSpec {
                    source: 2,
                    label: "B新".into(),
                    width: 20,
                },
                ColumnSpec {
                    source: 0,
                    label: "D".into(),
                    width: 18,
                },
                ColumnSpec {
                    source: 1,
                    label: "A".into(),
                    width: 24,
                },
            ],
        )
        .unwrap();

        let table = read_sheet_table(&path, "数据", 1).unwrap();
        assert_eq!(table.headers, vec!["B新", "D", "A"]);
        assert_eq!(table.rows[0].cells, vec!["b1", "", "a1"]);
        assert_eq!(table.rows[1].cells, vec!["b2", "", "a2"]);

        let _ = fs::remove_file(&path);
    }
}
