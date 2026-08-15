//! 使用统计导出的后端半边:通用 .xlsx 写入器 + 「全部」快速范围的最早记录日期。
//!
//! 架构切分(设计定稿):**前端出「工作簿规格」,这里只做无业务的逐格写入**——
//! sheet 名/表头/星期/粒度等全部文案由前端 i18n 解析好传入,本模块零语言知识、
//! 零统计口径知识,只认「带类型的单元格」。这样布局逻辑留在前端纯函数里
//! (vitest 可测),Rust 侧不随文案/口径变化而改动。
//!
//! 版式约定(观感优化定稿):数据表写成真正的 Excel Table(内置样式自带表头
//! 色带/斑马纹/筛选);时长用 `[h]:mm` 原生格式(仍是数字,可排序求和作图);
//! 默认网格线按 sheet 关闭;概览页走大标题 + 灰标签 + 斜体注脚的报告版式。

use rust_xlsxwriter::{
    ExcelDateTime, Format, FormatAlign, Table, TableColumn, TableStyle, Workbook,
};
use serde::Deserialize;
use tauri::State;

use crate::storage::{DbPool, SqliteResultExt};

/// 一个单元格。`t` 区分类型:
/// - s=文本 / b=加粗文本 / n=数字 / e=空占位
/// - d=日期("YYYY-MM-DD",真日期类型,可排序可作图表轴)
/// - dur=时长(值为**分钟**,写成 Excel 时间 + `[h]:mm` 格式,如 1249 → 20:49)
/// - pct=占比(值为 0..=1 小数,`0%` 格式)
/// - muted=灰色标签文本(概览页左列) / note=小字斜体注脚
#[derive(Debug, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "lowercase")]
pub enum Cell {
    S(String),
    B(String),
    N(f64),
    D(String),
    Dur(f64),
    Pct(f64),
    Muted(String),
    Note(String),
    E,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetSpec {
    pub name: String,
    /// 大标题:合并单元格 + 15pt 加粗,独占前两行(数据行随后)。概览页用。
    #[serde(default)]
    pub title: Option<String>,
    /// true = 把 rows(首行为表头)写成 Excel Table:内置样式的表头色带 +
    /// 斑马纹 + 筛选按钮。仅一行表头没有数据时自动退化为普通写入。
    #[serde(default)]
    pub table: bool,
    /// 关闭该 sheet 的屏幕网格线(Table/报告版式下更干净)
    #[serde(default)]
    pub hide_gridlines: bool,
    /// 冻结前 N 行 / 前 N 列(0 = 不冻结;含 title 偏移后的绝对行数)
    #[serde(default)]
    pub freeze_rows: u32,
    #[serde(default)]
    pub freeze_cols: u16,
    /// 非 Table sheet 上:首行按表头加粗 + 挂自动筛选(Table 自带,忽略此项)
    #[serde(default)]
    pub header_filter: bool,
    /// 每列宽度(字符单位);短于实际列数的部分用默认宽度
    #[serde(default)]
    pub column_widths: Vec<f64>,
    pub rows: Vec<Vec<Cell>>,
}

/// 「明细」sheet 的规格:原始活动记录由**后端直查直写**——十几万行经 IPC JSON
/// 转运是几十 MB 的荒谬绕路,这是对"前端出规格"架构的一次明确豁免。
/// 文案(sheet 名/表头/分类显示名)仍由前端传入,本模块保持零语言知识。
/// 口径与统计一致:应用显示名走分组,分组分类为 hidden 的行不导出。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawSheetSpec {
    pub name: String,
    /// 7 列表头:日期/开始时间/时长/应用/窗口标题/分类/设备
    pub headers: Vec<String>,
    /// "YYYY-MM-DD" 起止(含)
    pub start: String,
    pub end: String,
    /// 限定设备;None = 全部
    pub device_id: Option<String>,
    /// 分类 id → 当前语言显示名(找不到的按 id 原样)
    pub category_names: Vec<(String, String)>,
    /// 命中行数上限(Excel 104 万行)被截断时,追加在表末的说明
    pub truncated_note: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookSpec {
    pub sheets: Vec<SheetSpec>,
    #[serde(default)]
    pub raw: Option<RawSheetSpec>,
}

/// 明细一行(查询结果,写入前的中间形态)。
pub struct RawRow {
    date: String,
    /// "YYYY-MM-DDTHH:MM:SS"(已剥时区,Excel datetime 可直接解析)
    started: String,
    minutes: f64,
    app: String,
    title: String,
    category_id: String,
    device: String,
}

/// Excel 单表行数上限约 104.8 万;留出表头/注脚余量。
const RAW_ROW_CAP: usize = 1_000_000;

/// 分钟 → Excel 时间序列值(天为单位)。
fn minutes_to_excel(min: f64) -> f64 {
    min / (24.0 * 60.0)
}

/// 查明细行(同步,拿 rusqlite 连接直查——单测也走这里)。
/// 口径与 reports 的应用聚合一致:显示名走分组、hidden 分组整行排除;
/// 多取 1 行用于探测"是否超过上限"。
pub(crate) fn fetch_raw_rows(
    conn: &rusqlite::Connection,
    start: &str,
    end: &str,
    device_id: Option<&str>,
) -> rusqlite::Result<Vec<RawRow>> {
    let device_clause = if device_id.is_some() {
        "AND a.device_id = ?3"
    } else {
        ""
    };
    let sql = format!(
        "SELECT a.local_date,
                substr(a.started_at, 1, 19)                      AS started,
                a.duration_secs,
                COALESCE(g.display_name, a.process_name)         AS app,
                COALESCE(a.window_title, '')                     AS title,
                COALESCE(g.category_id, a.category_id, 'other')  AS cat,
                COALESCE(d.display_name, a.device_id)            AS device
         FROM activities a
         LEFT JOIN app_group_members gm
           ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
         LEFT JOIN app_groups g
           ON g.id = gm.group_id AND g.deleted_at IS NULL
         LEFT JOIN devices d
           ON d.device_id = a.device_id
         WHERE a.local_date >= ?1 AND a.local_date <= ?2 {device_clause}
           AND g.category_id IS NOT 'hidden'
           AND a.excluded = 0
         ORDER BY a.started_at
         LIMIT {}",
        RAW_ROW_CAP + 1
    );
    let mut stmt = conn.prepare(&sql)?;
    let map = |r: &rusqlite::Row| {
        Ok(RawRow {
            date: r.get(0)?,
            started: r.get(1)?,
            minutes: r.get::<_, i64>(2)? as f64 / 60.0,
            app: r.get(3)?,
            title: r.get(4)?,
            category_id: r.get(5)?,
            device: r.get(6)?,
        })
    };
    let it: Vec<RawRow> = if let Some(dev) = device_id {
        stmt.query_map(rusqlite::params![start, end, dev], map)?
            .collect::<rusqlite::Result<_>>()?
    } else {
        stmt.query_map(rusqlite::params![start, end], map)?
            .collect::<rusqlite::Result<_>>()?
    };
    Ok(it)
}

/// 把明细行写成 Table sheet(冻结表头、时长 [h]:mm、开始时间 hh:mm:ss)。
fn write_raw_sheet(
    wb: &mut Workbook,
    spec: &RawSheetSpec,
    mut rows: Vec<RawRow>,
) -> Result<(), String> {
    // headers 由前端经 IPC 传入(RawSheetSpec: Deserialize),属信任边界输入:
    // 空表头在 add_table 分支会让 `headers.len() - 1` 下溢(debug panic,
    // release 回绕成 u16::MAX 列),统一提前拒绝。
    if spec.headers.is_empty() {
        return Err("raw sheet: headers 不能为空".into());
    }
    let dur_fmt = Format::new().set_num_format("[h]:mm");
    let date_fmt = Format::new().set_num_format("yyyy-mm-dd");
    let time_fmt = Format::new().set_num_format("hh:mm:ss");
    let note_fmt = Format::new()
        .set_italic()
        .set_font_size(9)
        .set_font_color("#909090");
    let cat_name: std::collections::HashMap<&str, &str> = spec
        .category_names
        .iter()
        .map(|(id, name)| (id.as_str(), name.as_str()))
        .collect();

    let truncated = rows.len() > RAW_ROW_CAP;
    if truncated {
        rows.truncate(RAW_ROW_CAP);
    }

    let ws = wb.add_worksheet();
    ws.set_name(&spec.name).map_err(|e| e.to_string())?;
    ws.set_screen_gridlines(false);

    for (i, r) in rows.iter().enumerate() {
        let row = i as u32 + 1;
        if let Ok(dt) = ExcelDateTime::parse_from_str(&r.date) {
            ws.write_datetime_with_format(row, 0, &dt, &date_fmt)
                .map(|_| ())
                .map_err(|e| e.to_string())?;
        }
        if let Ok(dt) = ExcelDateTime::parse_from_str(&r.started) {
            ws.write_datetime_with_format(row, 1, &dt, &time_fmt)
                .map(|_| ())
                .map_err(|e| e.to_string())?;
        }
        ws.write_number_with_format(row, 2, minutes_to_excel(r.minutes), &dur_fmt)
            .map(|_| ())
            .map_err(|e| e.to_string())?;
        ws.write_string(row, 3, &r.app)
            .map(|_| ())
            .map_err(|e| e.to_string())?;
        ws.write_string(row, 4, &r.title)
            .map(|_| ())
            .map_err(|e| e.to_string())?;
        let cat = cat_name
            .get(r.category_id.as_str())
            .copied()
            .unwrap_or(r.category_id.as_str());
        ws.write_string(row, 5, cat)
            .map(|_| ())
            .map_err(|e| e.to_string())?;
        ws.write_string(row, 6, &r.device)
            .map(|_| ())
            .map_err(|e| e.to_string())?;
    }

    // Table 需要至少一行数据;空结果就只写普通表头
    if rows.is_empty() {
        for (c, h) in spec.headers.iter().enumerate() {
            ws.write_string_with_format(0, c as u16, h, &Format::new().set_bold())
                .map(|_| ())
                .map_err(|e| e.to_string())?;
        }
    } else {
        let columns: Vec<TableColumn> = spec
            .headers
            .iter()
            .map(|h| TableColumn::new().set_header(h))
            .collect();
        let table = Table::new()
            .set_style(TableStyle::Medium2)
            .set_columns(&columns);
        ws.add_table(
            0,
            0,
            rows.len() as u32,
            (spec.headers.len() - 1) as u16,
            &table,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())?;
    }
    if truncated {
        ws.write_string_with_format(rows.len() as u32 + 1, 0, &spec.truncated_note, &note_fmt)
            .map(|_| ())
            .map_err(|e| e.to_string())?;
    }
    ws.set_freeze_panes(1, 0)
        .map(|_| ())
        .map_err(|e| e.to_string())?;
    for (i, w) in [11.0, 9.5, 7.0, 22.0, 60.0, 10.0, 14.0].iter().enumerate() {
        ws.set_column_width(i as u16, *w)
            .map(|_| ())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn write_workbook(
    spec: &WorkbookSpec,
    raw_rows: Option<Vec<RawRow>>,
    path: &std::path::Path,
) -> Result<(), String> {
    let mut wb = Workbook::new();
    let bold = Format::new().set_bold();
    let title_fmt = Format::new()
        .set_bold()
        .set_font_size(15)
        .set_align(FormatAlign::VerticalCenter);
    let date_fmt = Format::new().set_num_format("yyyy-mm-dd");
    let dur_fmt = Format::new().set_num_format("[h]:mm");
    let pct_fmt = Format::new().set_num_format("0%");
    let muted_fmt = Format::new().set_font_color("#808080");
    let note_fmt = Format::new()
        .set_italic()
        .set_font_size(9)
        .set_font_color("#909090");

    for sheet in &spec.sheets {
        let ws = wb.add_worksheet();
        ws.set_name(&sheet.name).map_err(|e| e.to_string())?;
        if sheet.hide_gridlines {
            ws.set_screen_gridlines(false);
        }

        // 标题占第 0 行(合并到数据宽度),空一行,数据从第 2 行开始
        let offset: u32 = if let Some(title) = &sheet.title {
            let span = sheet
                .column_widths
                .len()
                .max(sheet.rows.iter().map(|r| r.len()).max().unwrap_or(1))
                .max(3) as u16;
            ws.merge_range(0, 0, 0, span - 1, title, &title_fmt)
                .map_err(|e| e.to_string())?;
            ws.set_row_height(0, 24.0).map_err(|e| e.to_string())?;
            2
        } else {
            0
        };

        let as_table = sheet.table && sheet.rows.len() >= 2 && !sheet.rows[0].is_empty();
        // Table 模式下表头由 Table 样式渲染,普通模式下按 header_filter 加粗
        let bold_header = !as_table && sheet.header_filter;

        for (ri, row) in sheet.rows.iter().enumerate() {
            let r = ri as u32 + offset;
            let header = bold_header && ri == 0;
            for (c, cell) in row.iter().enumerate() {
                let c = c as u16;
                let res = match cell {
                    Cell::S(v) if header => ws.write_string_with_format(r, c, v, &bold),
                    Cell::S(v) => ws.write_string(r, c, v),
                    Cell::B(v) => ws.write_string_with_format(r, c, v, &bold),
                    Cell::Muted(v) => ws.write_string_with_format(r, c, v, &muted_fmt),
                    Cell::Note(v) => ws.write_string_with_format(r, c, v, &note_fmt),
                    Cell::N(v) => ws.write_number(r, c, *v),
                    Cell::Dur(min) => {
                        ws.write_number_with_format(r, c, minutes_to_excel(*min), &dur_fmt)
                    }
                    Cell::Pct(v) => ws.write_number_with_format(r, c, *v, &pct_fmt),
                    Cell::D(v) => {
                        let dt = ExcelDateTime::parse_from_str(v).map_err(|e| e.to_string())?;
                        ws.write_datetime_with_format(r, c, &dt, &date_fmt)
                    }
                    Cell::E => continue,
                };
                res.map(|_| ()).map_err(|e| e.to_string())?;
            }
        }

        if as_table {
            let head = &sheet.rows[0];
            let columns: Vec<TableColumn> = head
                .iter()
                .map(|cell| {
                    let name = match cell {
                        Cell::S(v) | Cell::B(v) | Cell::Muted(v) | Cell::Note(v) => v.clone(),
                        _ => String::new(),
                    };
                    TableColumn::new().set_header(name)
                })
                .collect();
            let table = Table::new()
                .set_style(TableStyle::Medium2)
                .set_columns(&columns);
            ws.add_table(
                offset,
                0,
                offset + (sheet.rows.len() - 1) as u32,
                (head.len() - 1) as u16,
                &table,
            )
            .map(|_| ())
            .map_err(|e| e.to_string())?;
        } else if bold_header && !sheet.rows.is_empty() && sheet.rows.len() > 1 {
            let cols = sheet.rows[0].len();
            if cols > 0 {
                ws.autofilter(
                    offset,
                    0,
                    offset + (sheet.rows.len() - 1) as u32,
                    (cols - 1) as u16,
                )
                .map(|_| ())
                .map_err(|e| e.to_string())?;
            }
        }

        if sheet.freeze_rows > 0 || sheet.freeze_cols > 0 {
            ws.set_freeze_panes(sheet.freeze_rows, sheet.freeze_cols)
                .map(|_| ())
                .map_err(|e| e.to_string())?;
        }
        for (i, w) in sheet.column_widths.iter().enumerate() {
            ws.set_column_width(i as u16, *w)
                .map(|_| ())
                .map_err(|e| e.to_string())?;
        }
    }

    if let (Some(raw_spec), Some(rows)) = (&spec.raw, raw_rows) {
        write_raw_sheet(&mut wb, raw_spec, rows)?;
    }

    wb.save(path).map_err(|e| e.to_string())
}

/// 把工作簿规格写成 .xlsx。路径来自前端保存对话框(必须绝对路径)。
/// 带 `raw` 段时先直查明细行(不经 IPC),再连同规格一起写盘。
#[tauri::command]
pub async fn export_usage_xlsx(
    pool: State<'_, DbPool>,
    path: String,
    spec: WorkbookSpec,
) -> Result<(), String> {
    let p = std::path::PathBuf::from(&path);
    if !p.is_absolute() {
        return Err(format!("路径必须是绝对路径:{path}"));
    }
    let raw_rows = if let Some(raw) = &spec.raw {
        let (start, end, dev) = (raw.start.clone(), raw.end.clone(), raw.device_id.clone());
        Some(
            pool.0
                .call(move |conn| fetch_raw_rows(conn, &start, &end, dev.as_deref()).db())
                .await
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    tauri::async_runtime::spawn_blocking(move || write_workbook(&spec, raw_rows, &p))
        .await
        .map_err(|e| format!("spawn_blocking 失败:{e}"))?
}

/// 最早一条活动记录的本地日期("YYYY-MM-DD";空库为 None)。
/// 导出弹窗「全部」快速范围用它填起始日期。全设备口径。
#[tauri::command]
pub async fn earliest_activity_date(pool: State<'_, DbPool>) -> Result<Option<String>, String> {
    pool.0
        .call(|conn| {
            let v: Option<String> = conn
                .query_row("SELECT MIN(local_date) FROM activities", [], |r| r.get(0))
                .db()?;
            Ok(v)
        })
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 规格 → 文件落地:类型齐全(含 Table/时长/占比/标题)的工作簿写成合法 xlsx。
    #[test]
    fn writes_valid_xlsx_from_spec() {
        let spec = WorkbookSpec {
            sheets: vec![
                SheetSpec {
                    name: "概览".into(),
                    title: Some("Hindsight 使用统计".into()),
                    table: false,
                    hide_gridlines: true,
                    freeze_rows: 0,
                    freeze_cols: 0,
                    header_filter: false,
                    column_widths: vec![16.0, 28.0, 10.0],
                    rows: vec![
                        vec![
                            Cell::Muted("范围".into()),
                            Cell::S("2026-07-13 ~ 19".into()),
                        ],
                        vec![
                            Cell::B("分类".into()),
                            Cell::B("时长".into()),
                            Cell::B("占比".into()),
                        ],
                        vec![Cell::S("编程".into()), Cell::Dur(1249.0), Cell::Pct(0.41)],
                        vec![Cell::Note("说明:…".into())],
                    ],
                },
                SheetSpec {
                    name: "每日".into(),
                    title: None,
                    table: true,
                    hide_gridlines: true,
                    freeze_rows: 1,
                    freeze_cols: 1,
                    header_filter: false,
                    column_widths: vec![11.0, 8.0],
                    rows: vec![
                        vec![Cell::S("日期".into()), Cell::S("总计".into())],
                        vec![Cell::D("2026-07-13".into()), Cell::Dur(1249.0)],
                        vec![Cell::D("2026-07-14".into()), Cell::E],
                    ],
                },
            ],
            raw: None,
        };
        let dir = std::env::temp_dir().join("hindsight-xlsx-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("spec.xlsx");
        write_workbook(&spec, None, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        std::fs::remove_file(&path).ok();
    }

    /// raw 查询:分组显示名/hidden 排除/设备名回退,与统计口径一致。
    #[tokio::test]
    async fn fetch_raw_rows_respects_grouping_and_hidden() {
        let pool = crate::repo::test_util::fresh_test_pool().await;
        pool.0
            .call(|conn| {
                conn.execute_batch(
                    "INSERT INTO app_groups(id, display_name, category_id) VALUES
                       ('g1', 'QQ音乐', 'media'), ('g2', 'Secret', 'hidden');
                     INSERT INTO app_group_members(group_id, process_name) VALUES
                       ('g1', 'qqmusic.exe'), ('g2', 'secret.exe');
                     INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                       local_hour, process_name, window_title, category_id, device_id) VALUES
                       ('2026-07-18T10:00:00+09:00', '2026-07-18T10:05:00+09:00', 300,
                        '2026-07-18', 10, 'qqmusic.exe', '歌单', 'media', 'dev-a'),
                       ('2026-07-18T11:00:00+09:00', '2026-07-18T11:05:00+09:00', 300,
                        '2026-07-18', 11, 'secret.exe', '藏起来', 'hidden', 'dev-a'),
                       ('2026-07-19T09:00:00+09:00', '2026-07-19T09:10:00+09:00', 600,
                        '2026-07-19', 9, 'ungrouped.exe', NULL, 'other', 'dev-b');",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
                Ok(())
            })
            .await
            .unwrap();

        let rows = pool
            .0
            .call(|conn| Ok(fetch_raw_rows(conn, "2026-07-01", "2026-07-31", None).unwrap()))
            .await
            .unwrap();
        // hidden 分组整行排除;按 started_at 升序
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].app, "QQ音乐");
        assert_eq!(rows[0].started, "2026-07-18T10:00:00");
        assert_eq!(rows[0].minutes, 5.0);
        assert_eq!(rows[0].title, "歌单");
        // 未入组:显示名回退 process_name,标题 NULL → 空串,设备无记录回退 id
        assert_eq!(rows[1].app, "ungrouped.exe");
        assert_eq!(rows[1].title, "");
        assert_eq!(rows[1].device, "dev-b");
    }

    /// 前端 JSON 形态(adjacently tagged)能反序列化,新类型齐备。
    #[test]
    fn cell_json_shape_roundtrip() {
        let json = r#"{
          "sheets": [{
            "name": "S",
            "table": true,
            "hideGridlines": true,
            "rows": [[
              {"t":"s","v":"a"},{"t":"n","v":3},{"t":"d","v":"2026-01-02"},{"t":"e"},
              {"t":"b","v":"x"},{"t":"dur","v":90},{"t":"pct","v":0.5},
              {"t":"muted","v":"m"},{"t":"note","v":"i"}
            ]]
          }]
        }"#;
        let spec: WorkbookSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.sheets[0].rows[0].len(), 9);
        assert!(spec.sheets[0].table);
        assert!(matches!(spec.sheets[0].rows[0][5], Cell::Dur(v) if v == 90.0));
        assert!(matches!(spec.sheets[0].rows[0][6], Cell::Pct(v) if v == 0.5));
    }

    // ═════════════ write_raw_sheet ═════════════

    fn tmp_xlsx(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("hindsight-xlsx-test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{name}-{}.xlsx", std::process::id()))
    }

    /// 读 xlsx（zip 容器）里一个 entry 的全文。zip crate 是主 crate 现有依赖。
    fn zip_entry(path: &std::path::Path, name: &str) -> String {
        use std::io::Read;
        let mut z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut s = String::new();
        z.by_name(name)
            .unwrap_or_else(|e| panic!("xlsx 里应有 {name}: {e}"))
            .read_to_string(&mut s)
            .unwrap();
        s
    }

    fn zip_has_entry(path: &std::path::Path, name: &str) -> bool {
        let mut z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let found = z.by_name(name).is_ok();
        found
    }

    /// 只解压 entry 的前 `limit` 字节——超大 sheet XML（百万行）取头部看 dimension 就够。
    fn zip_entry_prefix(path: &std::path::Path, name: &str, limit: usize) -> String {
        use std::io::Read;
        let mut z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut f = z.by_name(name).unwrap();
        let mut buf = vec![0u8; limit];
        let mut n = 0;
        while n < limit {
            let k = f.read(&mut buf[n..]).unwrap();
            if k == 0 {
                break;
            }
            n += k;
        }
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    fn raw_spec(
        headers: Vec<String>,
        category_names: Vec<(String, String)>,
        note: &str,
    ) -> RawSheetSpec {
        RawSheetSpec {
            name: "明细".into(),
            headers,
            start: "2026-01-01".into(),
            end: "2026-12-31".into(),
            device_id: None,
            category_names,
            truncated_note: note.into(),
        }
    }

    fn seven_headers() -> Vec<String> {
        [
            "日期",
            "开始时间",
            "时长",
            "应用",
            "窗口标题",
            "分类",
            "设备",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    /// 真类型单元格：日期/开始时间是数值型 Excel 序列（非文本），时长走
    /// 分钟→天的时间序列 + `[h]:mm` 格式；分类名有映射用映射、缺失回退原 id；
    /// 数据落成真正的 Excel Table（表头进 table1.xml）。
    #[test]
    fn raw_sheet_writes_typed_cells_category_mapping_and_fallback() {
        let rows = vec![
            RawRow {
                date: "2026-07-18".into(),
                started: "2026-07-18T10:30:00".into(),
                minutes: 90.0,
                app: "QQ音乐".into(),
                title: "歌单".into(),
                category_id: "media".into(),
                device: "MacBook".into(),
            },
            RawRow {
                date: "2026-07-19".into(),
                started: "not-a-time".into(), // 解析失败 → 该格跳过，不 panic
                minutes: 0.0,
                app: "ungrouped.exe".into(),
                title: String::new(),
                category_id: "mystery-cat".into(), // 无映射 → 回退原 id
                device: "dev-b".into(),
            },
        ];
        let spec = raw_spec(
            seven_headers(),
            vec![("media".into(), "媒体".into())],
            "截断说明",
        );
        let mut wb = Workbook::new();
        write_raw_sheet(&mut wb, &spec, rows).unwrap();
        let path = tmp_xlsx("raw-typed");
        wb.save(&path).unwrap();

        let sheet = zip_entry(&path, "xl/worksheets/sheet1.xml");
        // 真日期类型：serial = 距 Excel 纪元 1899-12-30 的天数（数值单元格 <v>）
        let epoch = chrono::NaiveDate::from_ymd_opt(1899, 12, 30).unwrap();
        let serial = (chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap() - epoch).num_days();
        assert!(
            sheet.contains(&format!("<v>{serial}</v>")),
            "日期应写成 Excel 序列数值 {serial}"
        );
        // 开始时间 10:30 → serial + 0.4375
        assert!(
            sheet.contains(&format!("<v>{serial}.4375</v>")),
            "datetime 应写成 serial+小数"
        );
        // 时长 90 分钟 → 90/1440 = 0.0625（Excel 时间序列，仍是数字可排序求和）
        assert!(sheet.contains("<v>0.0625</v>"), "时长应是分钟/1440 的数值");

        // [h]:mm / hh:mm:ss 数字格式注册在 styles.xml
        let styles = zip_entry(&path, "xl/styles.xml");
        assert!(styles.contains("[h]:mm"), "时长格式 [h]:mm 应注册");
        assert!(styles.contains("hh:mm:ss"), "开始时间格式应注册");

        // 分类名映射 + 缺失回退
        let sst = zip_entry(&path, "xl/sharedStrings.xml");
        assert!(sst.contains("媒体"), "media 应映射为显示名");
        assert!(!sst.contains("<t>media</t>"), "已映射的原 id 不应再出现");
        assert!(sst.contains("mystery-cat"), "缺映射的分类应回退原 id");
        for s in ["QQ音乐", "歌单", "MacBook", "ungrouped.exe", "dev-b"] {
            assert!(sst.contains(s), "字符串列应含 {s}");
        }
        assert!(!sst.contains("截断说明"), "未超上限不得写截断注脚");

        // 真 Excel Table：表头进 table1.xml
        let table = zip_entry(&path, "xl/tables/table1.xml");
        for h in &spec.headers {
            assert!(table.contains(h.as_str()), "Table 应含表头 {h}");
        }
        std::fs::remove_file(&path).ok();
    }

    /// 空结果：退化为纯表头（无 Table part、无注脚），dimension 只有第 1 行。
    #[test]
    fn raw_sheet_empty_rows_writes_plain_header_no_table() {
        let spec = raw_spec(seven_headers(), vec![], "NOTE_MARK");
        let mut wb = Workbook::new();
        write_raw_sheet(&mut wb, &spec, vec![]).unwrap();
        let path = tmp_xlsx("raw-empty");
        wb.save(&path).unwrap();

        assert!(
            !zip_has_entry(&path, "xl/tables/table1.xml"),
            "空结果不应生成 Table part"
        );
        let sst = zip_entry(&path, "xl/sharedStrings.xml");
        for h in &spec.headers {
            assert!(sst.contains(h.as_str()), "普通表头应写出 {h}");
        }
        assert!(!sst.contains("NOTE_MARK"), "空结果未截断，不写注脚");
        let sheet = zip_entry(&path, "xl/worksheets/sheet1.xml");
        assert!(
            sheet.contains(r#"ref="A1:G1""#),
            "只有 7 格表头一行: {}",
            &sheet[..sheet.len().min(400)]
        );
        std::fs::remove_file(&path).ok();
    }

    /// 超 RAW_ROW_CAP 截断：100 万零 1 行进来 → 数据止步 100 万行 + 表末追加注脚。
    /// 表头 1 行 + 数据 1_000_000 行 + 注脚 1 行 = dimension 到 1000002。
    /// 注意：真写百万行，是本文件最重的一条（秒级）。
    #[test]
    fn raw_sheet_truncates_at_cap_and_appends_note() {
        let rows: Vec<RawRow> = (0..RAW_ROW_CAP + 1)
            .map(|_| RawRow {
                date: String::new(),    // 解析失败 → 跳过，省体积
                started: String::new(), // 同上
                minutes: 0.0,
                app: String::new(),
                title: String::new(),
                category_id: String::new(),
                device: String::new(),
            })
            .collect();
        let spec = raw_spec(seven_headers(), vec![], "NOTE_TRUNCATED_MARK");
        let mut wb = Workbook::new();
        write_raw_sheet(&mut wb, &spec, rows).unwrap();
        let path = tmp_xlsx("raw-cap");
        wb.save(&path).unwrap();

        let head = zip_entry_prefix(&path, "xl/worksheets/sheet1.xml", 2048);
        assert!(
            head.contains(r#"ref="A1:G1000002""#),
            "数据应止步 CAP（表头1 + 数据100万 + 注脚1 行）: {head}"
        );
        let sst = zip_entry(&path, "xl/sharedStrings.xml");
        assert!(sst.contains("NOTE_TRUNCATED_MARK"), "截断注脚应写在表末");
        std::fs::remove_file(&path).ok();
    }

    /// 回归(原 usize 下溢 bug):headers 为空时,add_table 分支的
    /// `(spec.headers.len() - 1) as u16` 曾在 debug 下 panic、release 下
    /// 回绕成 u16::MAX 列。修复后入口统一校验,rows 有无都干净返回 Err。
    #[test]
    fn raw_sheet_empty_headers_is_rejected_not_underflow() {
        let rows = vec![RawRow {
            date: "2026-07-18".into(),
            started: "2026-07-18T10:00:00".into(),
            minutes: 1.0,
            app: "a".into(),
            title: "t".into(),
            category_id: "c".into(),
            device: "d".into(),
        }];
        for rows in [rows, vec![]] {
            let spec = raw_spec(vec![], vec![], "");
            let mut wb = Workbook::new();
            let err = write_raw_sheet(&mut wb, &spec, rows)
                .expect_err("空 headers 应被入口校验拒绝(修复前是 panic/回绕)");
            assert!(err.contains("headers"), "错误信息应指明 headers: {err}");
        }
    }
}
