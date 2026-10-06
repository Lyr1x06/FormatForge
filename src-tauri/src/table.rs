//! 表格与结构化数据：XLSX ↔ CSV / TSV / JSON。
//!
//! 一个源文件可能产出多个文件——多工作表的 xlsx 转 CSV 时每个工作表一个输出。
//! 这类情况下主输出用 `spec.dst`，其余用「主名-工作表名」的兄弟路径。

use std::path::{Path, PathBuf};

use calamine::{Data, Reader};

use crate::job::{simple_error, CancelToken, JobSpec, JobState, Reporter};
use crate::paths;

/// 定界文本的方言
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Csv,
    Tsv,
}

impl Dialect {
    fn delimiter(self) -> u8 {
        match self {
            Dialect::Csv => b',',
            Dialect::Tsv => b'\t',
        }
    }

    fn from_format(id: &str) -> Option<Self> {
        match id {
            "csv" => Some(Dialect::Csv),
            "tsv" => Some(Dialect::Tsv),
            _ => None,
        }
    }
}

/// 一张表：首行是否表头 + 数据行
struct Sheet {
    name: String,
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

pub fn run(spec: &JobSpec, cancel: &CancelToken, reporter: &Reporter) {
    let result = dispatch(spec, cancel, reporter);

    match result {
        Ok(count) => {
            reporter.progress(spec.id, 1.0);
            let bytes_in = file_len(Path::new(&spec.src));
            let bytes_out = file_len(Path::new(&spec.dst));
            reporter.output(spec.id, &spec.dst, bytes_in, bytes_out, count as u64);
            reporter.state(spec.id, JobState::Done, None);
        }
        Err((kind, msg)) => {
            reporter.state(spec.id, JobState::Failed, Some(simple_error(kind, msg)));
        }
    }
}

/// 返回写出的文件数量
fn dispatch(
    spec: &JobSpec,
    cancel: &CancelToken,
    reporter: &Reporter,
) -> Result<usize, (&'static str, String)> {
    if cancel.is_cancelled() {
        return Err(("cancelled", String::new()));
    }
    reporter.state(spec.id, JobState::Converting, None);

    let src = PathBuf::from(&spec.src);
    let dst = PathBuf::from(&spec.dst);
    let sheet_mode = spec.text("sheet_mode", "separate");

    // 读源。xlsx 走 calamine（它会解析共享字符串与内联值），
    // 其余走文本读取。
    let mut sheets: Vec<Sheet> = match spec.src_format.as_str() {
        "xlsx" => read_xlsx(&src, &sheet_mode)?,
        "csv" | "tsv" => {
            let d = Dialect::from_format(&spec.src_format)
                .ok_or(("unsupported", "未知的定界文本格式".to_string()))?;
            vec![read_delimited(&src, d)?]
        }
        "json" => vec![read_json(&src)?],
        other => {
            return Err(("unsupported", format!("暂不支持读取 {other}")));
        }
    };

    if sheets.is_empty() {
        return Err(("corrupt", "文件里没有任何工作表".into()));
    }

    // 合并：把多张表叠成一张。
    //
    // 不能简单地首尾相接——各工作表的列可能不同（「华东」有 3 列、「华北」
    // 有 4 列），位置对齐会让数据整体错位。所以按**列名**并集化：列顺序取
    // 各表表头首次出现的顺序，每张表的每行按自己的表头名填进对应的列，
    // 没有的列留空。
    //
    // 工作表没有可用表头名时（整行都是空的）退回位置对齐，否则那张表的
    // 数据会全部落进一个空列名里、等于丢掉。
    if sheet_mode == "merge" && sheets.len() > 1 {
        let mut columns: Vec<String> = Vec::new();
        for s in &sheets {
            for h in &s.header {
                if !h.is_empty() && !columns.iter().any(|c| c == h) {
                    columns.push(h.clone());
                }
            }
        }

        let mut rows: Vec<Vec<String>> = Vec::new();
        for s in &sheets {
            // 表头名 → 并集里的列号。整张表都没有可用列名时返回 None，
            // 表示走位置对齐。
            let mapping: Option<Vec<Option<usize>>> = {
                let named = s.header.iter().filter(|h| !h.is_empty()).count();
                if named == 0 {
                    None
                } else {
                    Some(
                        s.header
                            .iter()
                            .map(|h| {
                                if h.is_empty() {
                                    None
                                } else {
                                    columns.iter().position(|c| c == h)
                                }
                            })
                            .collect(),
                    )
                }
            };

            for r in &s.rows {
                let mut row = vec![String::new(); columns.len()];
                match &mapping {
                    Some(map) => {
                        for (i, cell) in r.iter().enumerate() {
                            if let Some(Some(col)) = map.get(i) {
                                row[*col] = cell.clone();
                            }
                        }
                    }
                    None => {
                        for (i, cell) in r.iter().enumerate().take(columns.len()) {
                            row[i] = cell.clone();
                        }
                    }
                }
                rows.push(row);
            }
        }

        let name = sheets
            .first()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        sheets = vec![Sheet { name, header: columns, rows }];
    }

    reporter.progress(spec.id, 0.4);

    if cancel.is_cancelled() {
        return Err(("cancelled", String::new()));
    }

    // 写目标。单表时直接用目标路径；多表时加工作表名后缀，
    // 表名里不能进文件名系统的字符替换掉。
    let multi = sheets.len() > 1;
    let mut written = 0usize;

    for (i, sheet) in sheets.iter().enumerate() {
        let out = if multi {
            sibling_with_suffix(&dst, &sanitize(&sheet.name))
        } else {
            dst.clone()
        };

        let tmp = temp_sibling(&out);
        let r = match spec.dst_format.as_str() {
            "csv" => write_delimited(&sheet, &tmp, Dialect::Csv, spec),
            "tsv" => write_delimited(&sheet, &tmp, Dialect::Tsv, spec),
            "json" => write_json(&sheet, &tmp, spec),
            "xlsx" => write_xlsx(std::slice::from_ref(sheet), &tmp),
            other => Err(("unsupported", format!("暂不支持写出 {other}"))),
        };

        if let Err((k, m)) = r {
            let _ = std::fs::remove_file(paths::for_io(&tmp));
            return Err((k, m));
        }
        if let Err(e) = std::fs::rename(paths::for_io(&tmp), paths::for_io(&out)) {
            let _ = std::fs::remove_file(paths::for_io(&tmp));
            return Err(("io", format!("无法落地输出文件：{e}")));
        }
        written += 1;
        reporter.progress(spec.id, 0.4 + 0.6 * ((i + 1) as f32 / sheets.len() as f32));
    }

    Ok(written)
}

/* ---------------------------------------------------------------- 读 */

fn read_xlsx(src: &Path, sheet_mode: &str) -> Result<Vec<Sheet>, (&'static str, String)> {
    use calamine::open_workbook_auto;

    let mut wb = open_workbook_auto(paths::for_io(src))
        .map_err(|e| classify_calamine(&e.to_string()))?;

    let names = wb.sheet_names().to_vec();
    if names.is_empty() {
        return Err(("corrupt", "工作簿里没有工作表".into()));
    }

    // sheet_mode 只在这里生效（转 PDF 那条路在 Office 泳道，Excel 自己
    // 决定哪个工作表落到哪一页，轮不到我们挑）。
    //
    // 这里只处理「只取第一个」；「合并」是读完之后把几张表叠起来的事，
    // 统一放在 dispatch 里做，那样 csv / json 源也走同一条路。
    let names: Vec<String> = if sheet_mode == "first" {
        names.into_iter().take(1).collect()
    } else {
        names
    };

    let mut out = Vec::new();
    for name in names {
        let range = wb
            .worksheet_range(&name)
            .map_err(|e| classify_calamine(&e.to_string()))?;

        let mut rows_iter = range.rows();
        let header: Vec<String> = rows_iter
            .next()
            .map(|r| r.iter().map(cell_text).collect())
            .unwrap_or_default();
        let rows: Vec<Vec<String>> = rows_iter
            .map(|r| r.iter().map(cell_text).collect())
            .collect();

        out.push(Sheet { name, header, rows });
    }
    Ok(out)
}

/// calamine 的单元格 → 统一按文本处理。
///
/// 数字保留可读形式：`1.0` 要写成 `1`，否则 CSv 里满屏浮点尾巴。
fn cell_text(c: &Data) -> String {
    match c {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => fmt_float(*f),
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(dt) => dt.to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("{e:?}"),
    }
}

fn fmt_float(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        // 去掉多余的尾零，但保留有效小数
        let s = format!("{f}");
        s
    }
}

fn read_delimited(src: &Path, dialect: Dialect) -> Result<Sheet, (&'static str, String)> {
    let raw = std::fs::read(paths::for_io(src))
        .map_err(|e| ("io", format!("无法读取源文件：{e}")))?;
    let text = decode(&raw);

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(dialect.delimiter())
        .flexible(true)
        // 自己管表头——默认的 has_headers(true) 会让 records() 跳过第一行，
        // 那样我再取一次 header 就吃掉了一行数据
        .has_headers(false)
        .from_reader(text.as_bytes());

    let mut records = rdr.records();
    let header: Vec<String> = records
        .next()
        .and_then(|r| r.ok())
        .map(|r| r.iter().map(String::from).collect())
        .unwrap_or_default();

    let rows: Vec<Vec<String>> = records
        .filter_map(|r| r.ok())
        .map(|r| r.iter().map(String::from).collect())
        .collect();

    Ok(Sheet { name: "Sheet1".into(), header, rows })
}

fn read_json(src: &Path) -> Result<Sheet, (&'static str, String)> {
    let raw = std::fs::read(paths::for_io(src))
        .map_err(|e| ("io", format!("无法读取源文件：{e}")))?;
    let text = decode(&raw);

    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| ("corrupt", format!("JSON 解析失败：{e}")))?;

    let arr = match &value {
        serde_json::Value::Array(a) => a,
        // 顶层是对象时，如果它有唯一一个数组字段就取那一个
        serde_json::Value::Object(o) => o
            .values()
            .find_map(|v| v.as_array())
            .ok_or(("unsupported", "JSON 顶层既不是数组，也没有数组字段".to_string()))?,
        _ => return Err(("unsupported", "JSON 顶层既不是数组也不是对象".into())),
    };

    // 对象数组 → 用键当表头；值数组 → 用序号当表头
    let mut header: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();

    for item in arr {
        match item {
            serde_json::Value::Object(map) => {
                if header.is_empty() {
                    header = map.keys().cloned().collect();
                }
                rows.push(
                    header
                        .iter()
                        .map(|k| map.get(k).map(json_scalar).unwrap_or_default())
                        .collect(),
                );
            }
            serde_json::Value::Array(vals) => {
                if header.is_empty() {
                    header = (1..=vals.len()).map(|i| format!("列{i}")).collect();
                }
                rows.push(vals.iter().map(json_scalar).collect());
            }
            other => {
                // 标量数组当成单列
                if header.is_empty() {
                    header = vec!["值".into()];
                }
                rows.push(vec![json_scalar(other)]);
            }
        }
    }

    if header.is_empty() {
        return Err(("corrupt", "JSON 里没有可提取的记录".into()));
    }

    Ok(Sheet { name: "Sheet1".into(), header, rows })
}

fn json_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        // 嵌套结构原样序列化，至少不丢数据
        other => other.to_string(),
    }
}

/// 按参数指定的编码解码，默认 UTF-8，带 BOM 时自动识别。
fn decode(raw: &[u8]) -> String {
    // BOM 优先
    if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&raw[3..]).into_owned();
    }
    if raw.starts_with(&[0xFF, 0xFE]) || raw.starts_with(&[0xFE, 0xFF]) {
        let (text, _, _) = encoding_rs::UTF_16LE.decode(raw);
        return text.into_owned();
    }
    // 合法 UTF-8 就直接用
    if let Ok(s) = std::str::from_utf8(raw) {
        return s.to_string();
    }
    // 否则按 GBK 试——中文 Windows 上导出的 CSV 十有八九是 GBK
    let (text, _, _) = encoding_rs::GBK.decode(raw);
    text.into_owned()
}

fn encode(text: &str, encoding: &str) -> Vec<u8> {
    match encoding {
        "utf-8-bom" => {
            let mut out = vec![0xEF, 0xBB, 0xBF];
            out.extend_from_slice(text.as_bytes());
            out
        }
        "gbk" => {
            let (bytes, _, _) = encoding_rs::GBK.encode(text);
            bytes.into_owned()
        }
        _ => text.as_bytes().to_vec(),
    }
}

/* ---------------------------------------------------------------- 写 */

fn write_delimited(
    sheet: &Sheet,
    path: &Path,
    dialect: Dialect,
    spec: &JobSpec,
) -> Result<(), (&'static str, String)> {
    let mut wtr = csv::WriterBuilder::new()
        .delimiter(dialect.delimiter())
        .from_writer(Vec::new());

    wtr.write_record(&sheet.header)
        .map_err(|e| ("encode", format!("写入表头失败：{e}")))?;
    for row in &sheet.rows {
        wtr.write_record(row)
            .map_err(|e| ("encode", format!("写入数据行失败：{e}")))?;
    }
    let body = wtr
        .into_inner()
        .map_err(|e| ("encode", format!("收尾失败：{e}")))?;
    let text = String::from_utf8_lossy(&body).into_owned();

    let encoding = spec.text("encoding", "utf-8");
    std::fs::write(paths::for_io(path), encode(&text, encoding))
        .map_err(|e| ("io", format!("写入失败：{e}")))
}

fn write_json(
    sheet: &Sheet,
    path: &Path,
    spec: &JobSpec,
) -> Result<(), (&'static str, String)> {
    let use_header = spec.flag("header_row", true);
    let mut out: Vec<serde_json::Value> = Vec::with_capacity(sheet.rows.len());

    for row in &sheet.rows {
        if use_header && !sheet.header.is_empty() {
            // 键值对，数值型看起来像数字就还原成数字
            let mut map = serde_json::Map::new();
            for (i, key) in sheet.header.iter().enumerate() {
                let raw = row.get(i).cloned().unwrap_or_default();
                map.insert(key.clone(), coerce(&raw));
            }
            out.push(serde_json::Value::Object(map));
        } else {
            out.push(serde_json::Value::Array(
                row.iter().map(|s| coerce(s)).collect(),
            ));
        }
    }

    let text = serde_json::to_string_pretty(&out)
        .map_err(|e| ("encode", format!("JSON 序列化失败：{e}")))?;

    let encoding = spec.text("encoding", "utf-8");
    std::fs::write(paths::for_io(path), encode(&text, encoding))
        .map_err(|e| ("io", format!("写入失败：{e}")))
}

/// 看起来是数字的还原成 JSON 数字，空串还原成 null。
/// 全程按字符串输出会让下游没法直接做算术。
///
/// 但**不能无脑转**：带前导零（`007`）、超长（身份证、银行卡）、
/// 带 `+` 前缀（电话号码）的都不是数，转了就永久丢信息。
fn coerce(s: &str) -> serde_json::Value {
    let t = s.trim();
    if t.is_empty() {
        return serde_json::Value::Null;
    }
    if t.eq_ignore_ascii_case("true") {
        return serde_json::Value::Bool(true);
    }
    if t.eq_ignore_ascii_case("false") {
        return serde_json::Value::Bool(false);
    }
    if looks_like_text(t) {
        return serde_json::Value::String(s.to_string());
    }
    if let Ok(i) = t.parse::<i64>() {
        return serde_json::Value::Number(i.into());
    }
    if let Ok(f) = t.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return serde_json::Value::Number(n);
        }
    }
    serde_json::Value::String(s.to_string())
}

fn write_xlsx(sheets: &[Sheet], path: &Path) -> Result<(), (&'static str, String)> {
    use rust_xlsxwriter::Workbook;

    let mut wb = Workbook::new();

    for sheet in sheets {
        let ws = wb.add_worksheet();
        ws.set_name(&sheet.name)
            .map_err(|e| ("encode", format!("工作表名无效：{e}")))?;

        if !sheet.header.is_empty() {
            for (c, h) in sheet.header.iter().enumerate() {
                ws.write_string(0, c as u16, h)
                    .map_err(|e| ("encode", format!("写入失败：{e}")))?;
            }
        }

        let offset = if sheet.header.is_empty() { 0 } else { 1 };
        for (r, row) in sheet.rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                if cell.is_empty() {
                    continue;
                }
                let row_idx = (r + offset) as u32;
                // 数值型写成数字，Excel 里才能直接参与计算
                match cell.parse::<f64>() {
                    Ok(n) if !looks_like_text(cell) => {
                        ws.write_number(row_idx, c as u16, n)
                            .map_err(|e| ("encode", format!("写入失败：{e}")))?;
                    }
                    _ => {
                        ws.write_string(row_idx, c as u16, cell)
                            .map_err(|e| ("encode", format!("写入失败：{e}")))?;
                    }
                }
            }
        }
    }

    wb.save(paths::for_io(path))
        .map_err(|e| ("io", format!("保存 xlsx 失败：{e}")))
}

/// 带前导零或过长的一串数字（电话号码、身份证、编号）不能当数字，
/// 否则 Excel 会把 007 变成 7。
fn looks_like_text(s: &str) -> bool {
    let t = s.trim();
    (t.len() > 1 && t.starts_with('0') && !t.starts_with("0."))
        || t.len() > 15
        || t.starts_with('+')
}

fn classify_calamine(msg: &str) -> (&'static str, String) {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("password") || msg.contains("密码") || msg.contains("加密") {
        ("password", "工作簿已加密，需要密码".into())
    } else if lower.contains("zip") || lower.contains("invalid") || lower.contains("format") {
        ("corrupt", format!("文件已损坏或不是有效的表格：{msg}"))
    } else if lower.contains("no such file") || lower.contains("not found") {
        ("not_found", "找不到源文件".into())
    } else {
        ("unknown", msg.to_string())
    }
}

/* ---------------------------------------------------------------- 小工具 */

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(paths::for_io(p)).map(|m| m.len()).unwrap_or(0)
}

fn temp_sibling(dst: &Path) -> PathBuf {
    let ext = dst.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let name = dst.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    dst.with_file_name(format!("{name}.{ext}.fftmp"))
}

/// 在主名与扩展名之间插一个后缀：`报告.xlsx` → `报告-华东.csv`
fn sibling_with_suffix(dst: &Path, suffix: &str) -> PathBuf {
    let stem = dst.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = dst.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    dst.with_file_name(format!("{stem}-{suffix}.{ext}"))
}

/// 工作表名转成合法文件名片段
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() { "Sheet".into() } else { trimmed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_without_noise() {
        assert_eq!(fmt_float(1.0), "1");
        assert_eq!(fmt_float(-3.0), "-3");
        assert_eq!(fmt_float(1.5), "1.5");
        assert_eq!(fmt_float(0.0), "0");
    }

    #[test]
    fn keeps_leading_zeros_as_text() {
        assert!(looks_like_text("007"));
        assert!(looks_like_text("+8613800000000"));
        assert!(looks_like_text("1234567890123456789"));
        assert!(!looks_like_text("0.5"));
        assert!(!looks_like_text("42"));
    }

    #[test]
    fn sanitizes_sheet_names() {
        assert_eq!(sanitize("华东/华南"), "华东_华南");
        assert_eq!(sanitize("Sheet1"), "Sheet1");
        assert_eq!(sanitize("///"), "___");
    }

    #[test]
    fn coerces_scalars() {
        assert_eq!(coerce("42"), serde_json::json!(42));
        assert_eq!(coerce("1.5"), serde_json::json!(1.5));
        assert_eq!(coerce(""), serde_json::Value::Null);
        assert_eq!(coerce("true"), serde_json::json!(true));
        assert_eq!(coerce("华东"), serde_json::json!("华东"));
    }

    #[test]
    fn coerce_keeps_identifiers_as_strings() {
        // 转成数字会永久丢信息，必须原样保留
        assert_eq!(coerce("007"), serde_json::json!("007"));
        assert_eq!(coerce("+8613800000000"), serde_json::json!("+8613800000000"));
        assert_eq!(
            coerce("1234567890123456789"),
            serde_json::json!("1234567890123456789")
        );
        // 前导零的小数仍然是数
        assert_eq!(coerce("0.5"), serde_json::json!(0.5));
    }

    #[test]
    fn decodes_gbk_fallback() {
        // 「中文」的 GBK 编码
        let gbk = [0xD6, 0xD0, 0xCE, 0xC4];
        assert_eq!(decode(&gbk), "中文");
    }
}
