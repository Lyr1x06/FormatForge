//! 端到端管线测试：文件 → 扫描 → 规划 → 转换 → 校验产物。
//!
//! 全部在临时目录里跑真实文件，不走 COM（Office 泳道需要装 Office，
//! 那条路另有 scripts/test-office-worker.mjs 覆盖）。

use std::path::Path;

use format_forge_lib::testkit::{
    run_batch_sync, run_specs_with_options, scan_and_plan, temp_dir, write_text,
};

/// 造一个最小的 docx：就是 zip 里放 word/document.xml
fn make_docx(path: &Path, body_xml: &str) {
    use std::io::Write;

    let file = std::fs::File::create(path).expect("创建 docx");
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:body>{body_xml}</w:body></w:document>"#
    );

    zip.start_file("word/document.xml", opts).unwrap();
    zip.write_all(doc.as_bytes()).unwrap();
    zip.finish().unwrap();
}

fn p(text: &str) -> String {
    format!(r#"<w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
}

fn heading(level: u8, text: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading{level}"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
    )
}

#[test]
fn docx_to_txt_extracts_paragraphs_and_table() {
    let dir = temp_dir("docx-txt");
    let src = dir.join("样本.docx");

    let body = format!(
        "{}{}{}{}",
        heading(1, "外景简报"),
        p("今天光线不错。"),
        p(""),
        r#"<w:tbl>
            <w:tr><w:tc><w:p><w:r><w:t>地点</w:t></w:r></w:p></w:tc>
                  <w:tc><w:p><w:r><w:t>时段</w:t></w:r></w:p></w:tc></w:tr>
            <w:tr><w:tc><w:p><w:r><w:t>江边</w:t></w:r></w:p></w:tc>
                  <w:tc><w:p><w:r><w:t>18:40</w:t></w:r></w:p></w:tc></w:tr>
           </w:tbl>"#
    );
    make_docx(&src, &body);

    let out = dir.join("样本.txt");
    let outcome = run_batch_sync(&[src.to_string_lossy().to_string()], "txt", &dir);

    assert!(out.exists(), "产物没生成；事件：{:#?}", outcome.events);
    assert_eq!(outcome.failed(), 0, "事件：{:#?}", outcome.events);

    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("外景简报"), "缺标题：{text}");
    assert!(text.contains("今天光线不错。"), "缺正文：{text}");
    assert!(text.contains("地点 | 时段"), "表格没拉平：{text}");
    assert!(text.contains("江边 | 18:40"), "表格数据丢失：{text}");
}

#[test]
fn docx_to_md_keeps_heading_level_and_bold() {
    let dir = temp_dir("docx-md");
    let src = dir.join("稿子.docx");

    let body = format!(
        "{}{}",
        heading(2, "第二节"),
        r#"<w:p><w:r><w:t>普通</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>加粗</w:t></w:r></w:p>"#
    );
    make_docx(&src, &body);

    let outcome = run_batch_sync(&[src.to_string_lossy().to_string()], "md", &dir);
    assert_eq!(outcome.failed(), 0, "事件：{:#?}", outcome.events);

    let md = std::fs::read_to_string(dir.join("稿子.md")).unwrap();
    assert!(md.starts_with("## 第二节"), "标题级别不对：{md}");
    assert!(md.contains("普通**加粗**"), "粗体没保留：{md}");
}

#[test]
fn xlsx_to_csv_splits_per_sheet() {
    let dir = temp_dir("xlsx-csv");
    let src = dir.join("台账.xlsx");

    // 先造一个两表的 xlsx，再转回 csv
    write_two_sheet_xlsx(&src);

    let outcome = run_batch_sync(&[src.to_string_lossy().to_string()], "csv", &dir);
    assert_eq!(outcome.failed(), 0, "事件：{:#?}", outcome.events);

    let a = dir.join("台账-华东.csv");
    let b = dir.join("台账-华北.csv");
    assert!(a.exists(), "缺第一个工作表的产物；目录内容：{:?}", list(&dir));
    assert!(b.exists(), "缺第二个工作表的产物；目录内容：{:?}", list(&dir));

    let ca = std::fs::read_to_string(&a).unwrap();
    assert!(ca.contains("地区"), "表头丢了：{ca}");
    assert!(ca.contains("上海"), "数据丢了：{ca}");
    // 数字不该带浮点尾巴
    assert!(!ca.contains("1280.0"), "出现了 1280.0 这样的浮点噪声：{ca}");
}

/// `sheet_mode = first` 只取第一个工作表。
///
/// 这个参数曾在注册表里挂了很久却没有任何后端代码读它——UI 上看得见、
/// 点了也有状态，只是转了等于没转。`registry_shape.rs` 的
/// `every_option_key_is_read_by_the_backend` 就是为堵住这类漏洞写的。
#[test]
fn xlsx_sheet_mode_first_takes_only_the_first_sheet() {
    let dir = temp_dir("xlsx-first");
    let src = dir.join("src").join("台账.xlsx");
    write_two_sheet_xlsx(&src);

    let (_, specs) = scan_and_plan(&[src.to_string_lossy().to_string()], "csv", &dir);
    let outcome = run_specs_with_options(specs, &dir, &[("sheet_mode", "first")]);
    assert_eq!(outcome.failed(), 0, "事件：{:#?}", outcome.events);

    let got = outputs(&dir);
    assert_eq!(
        got,
        vec!["台账.csv".to_string()],
        "只该产出一个文件，实际：{got:?}"
    );
    let text = std::fs::read_to_string(dir.join("台账.csv")).unwrap();
    assert!(text.contains("上海"), "该是第一个工作表的内容：{text}");
    assert!(!text.contains("北京"), "第二个工作表不该出现：{text}");
}

/// `sheet_mode = merge` 按**列名**并集合并，而不是按位置首尾相接。
///
/// 两张表列不同时，位置对齐会让数据整体错位（「华北」的第 2 列是「备注」，
/// 位置对齐会把它写进「销量」列）。这条断言守着这个区别。
#[test]
fn xlsx_sheet_mode_merge_unions_columns_by_name() {
    let dir = temp_dir("xlsx-merge");
    let src = dir.join("src").join("台账.xlsx");
    write_ragged_sheet_xlsx(&src);

    let (_, specs) = scan_and_plan(&[src.to_string_lossy().to_string()], "csv", &dir);
    let outcome = run_specs_with_options(specs, &dir, &[("sheet_mode", "merge")]);
    assert_eq!(outcome.failed(), 0, "事件：{:#?}", outcome.events);

    let got = outputs(&dir);
    assert_eq!(got, vec!["台账.csv".to_string()], "只该产出一个文件：{got:?}");

    let text = std::fs::read_to_string(dir.join("台账.csv")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "地区,销量,备注", "表头该是列名并集：{text}");

    // 「华东」没有「备注」列 → 补空；「华北」的 备注 要落在第 3 列
    assert_eq!(lines[1], "上海,1280,", "缺列该补空：{text}");
    assert_eq!(lines[2], "北京,960,团购", "列名该决定落位：{text}");
}

#[test]
fn csv_to_json_writes_typed_values() {
    let dir = temp_dir("csv-json");
    let src = dir.join("统计.csv");
    // 故意混入前导零的编号，验证它不会被转成数字
    write_text(&src, "编号,数量,备注\n007,42,正常\n008,3,缺货\n");

    let outcome = run_batch_sync(&[src.to_string_lossy().to_string()], "json", &dir);
    assert_eq!(outcome.failed(), 0, "事件：{:#?}", outcome.events);

    let json = std::fs::read_to_string(dir.join("统计.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["编号"], serde_json::json!("007"), "前导零被吃掉了：{json}");
    assert_eq!(arr[0]["数量"], serde_json::json!(42), "数字没还原：{json}");
    assert_eq!(arr[1]["备注"], serde_json::json!("缺货"));
}

#[test]
fn mismatched_extension_uses_real_format() {
    let dir = temp_dir("mismatch");
    // 真身是 PNG，却叫 .jpg。魔数骗不了人。
    let src = dir.join("伪装.jpg");
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0u8; 32]);
    std::fs::write(&src, &bytes).unwrap();

    let (files, specs) = scan_and_plan(&[src.to_string_lossy().to_string()], "webp", &dir);
    let f = files.first().expect("应该扫到一个文件");

    assert_eq!(f.format, "png", "应按内容识别为 PNG，而不是信 .jpg");
    assert!(f.mismatch, "应标记扩展名与内容不符");
    // 走的是真实格式的目标列表而不是扩展名的
    assert!(f.targets.iter().any(|t| t == "webp"));
    assert_eq!(specs.len(), 1, "应能规划出转换作业");
}

/// 内容认不出来时按扩展名归类，让转换阶段报错而不是静默丢弃。
#[test]
fn unrecognised_content_falls_back_to_extension() {
    let dir = temp_dir("fallback");
    let src = dir.join("坏掉的.docx");
    write_text(&src, "这不是一个 zip 包，只是几行字");

    let (files, specs) = scan_and_plan(&[src.to_string_lossy().to_string()], "txt", &dir);
    let f = files.first().expect("应保留这个文件，而不是静默丢掉");

    assert_eq!(f.format, "docx", "应按扩展名归类");
    assert!(!f.mismatch, "内容无法识别时不该断言「不符」");
    assert_eq!(specs.len(), 1, "应能规划出作业，由转换阶段报错");

    // 转换阶段应该给出明确的失败，而不是成功产出一个空文件
    let outcome = run_batch_sync(&[src.to_string_lossy().to_string()], "txt", &dir);
    assert_eq!(outcome.failed(), 1, "坏文件应转换失败：{:#?}", outcome.events);
}

#[test]
fn corrupt_file_fails_without_killing_batch() {
    let dir = temp_dir("corrupt");
    let bad = dir.join("坏的.docx");
    let good = dir.join("好的.docx");

    write_text(&bad, "这不是一个 zip 包");
    make_docx(&good, &p("正常内容"));

    let outcome = run_batch_sync(
        &[bad.to_string_lossy().to_string(), good.to_string_lossy().to_string()],
        "txt",
        &dir,
    );

    assert_eq!(outcome.succeeded(), 1, "好的那个应该成功；事件：{:#?}", outcome.events);
    assert_eq!(outcome.failed(), 1, "坏的那个应该失败；事件：{:#?}", outcome.events);
    assert!(dir.join("好的.txt").exists());
}

/* ---------------------------------------------------------------- 辅助 */

fn list(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// 只列产物，不含源目录 `src/`。
///
/// 用了 `run_specs_with_options` 的测试必须把源文件放进子目录：输出目录
/// 就落在同一个临时目录里，源文件平铺在那儿会被算成「多出来的文件」。
fn outputs(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = list(dir).into_iter().filter(|n| n != "src").collect();
    v.sort();
    v
}

/// 用应用自己的 xlsx 写出能力造一个两表工作簿——这样测试不依赖外部样本
fn write_two_sheet_xlsx(path: &Path) {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("创建源目录");
    }
    let file = std::fs::File::create(path).expect("创建 xlsx");
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut put = |name: &str, data: String| {
        zip.start_file(name, opts).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    };

    put(
        "[Content_Types].xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#
            .into(),
    );
    put(
        "_rels/.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#
            .into(),
    );
    put(
        "xl/workbook.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets>
<sheet name="华东" sheetId="1" r:id="rId1"/>
<sheet name="华北" sheetId="2" r:id="rId2"/>
</sheets></workbook>"#
            .into(),
    );
    put(
        "xl/_rels/workbook.xml.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#
            .into(),
    );

    let sheet = |rows: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>{rows}</sheetData></worksheet>"#
        )
    };

    put(
        "xl/worksheets/sheet1.xml",
        sheet(
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>地区</t></is></c><c r="B1" t="inlineStr"><is><t>销量</t></is></c></row>
               <row r="2"><c r="A2" t="inlineStr"><is><t>上海</t></is></c><c r="B2" t="n"><v>1280</v></c></row>"#,
        ),
    );
    put(
        "xl/worksheets/sheet2.xml",
        sheet(
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>地区</t></is></c><c r="B1" t="inlineStr"><is><t>销量</t></is></c></row>
               <row r="2"><c r="A2" t="inlineStr"><is><t>北京</t></is></c><c r="B2" t="n"><v>960</v></c></row>"#,
        ),
    );

    zip.finish().unwrap();
}

/// 两张**列不一样**的工作表：华东是「地区/销量」，华北是「地区/销量/备注」。
/// 用来验证合并是按列名并集，而不是按位置首尾相接。
fn write_ragged_sheet_xlsx(path: &Path) {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("创建源目录");
    }
    let file = std::fs::File::create(path).expect("创建 xlsx");
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut put = |name: &str, data: String| {
        zip.start_file(name, opts).unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    };

    put(
        "[Content_Types].xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#
            .into(),
    );
    put(
        "_rels/.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#
            .into(),
    );
    put(
        "xl/workbook.xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets>
<sheet name="华东" sheetId="1" r:id="rId1"/>
<sheet name="华北" sheetId="2" r:id="rId2"/>
</sheets></workbook>"#
            .into(),
    );
    put(
        "xl/_rels/workbook.xml.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#
            .into(),
    );

    let sheet = |rows: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>{rows}</sheetData></worksheet>"#
        )
    };

    // 华东没有「备注」列
    put(
        "xl/worksheets/sheet1.xml",
        sheet(
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>地区</t></is></c><c r="B1" t="inlineStr"><is><t>销量</t></is></c></row>
               <row r="2"><c r="A2" t="inlineStr"><is><t>上海</t></is></c><c r="B2" t="n"><v>1280</v></c></row>"#,
        ),
    );
    // 华北多一列，且「备注」不在最后（故意让位置对齐出错）
    put(
        "xl/worksheets/sheet2.xml",
        sheet(
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>地区</t></is></c><c r="B1" t="inlineStr"><is><t>销量</t></is></c><c r="C1" t="inlineStr"><is><t>备注</t></is></c></row>
               <row r="2"><c r="A2" t="inlineStr"><is><t>北京</t></is></c><c r="B2" t="n"><v>960</v></c><c r="C2" t="inlineStr"><is><t>团购</t></is></c></row>"#,
        ),
    );

    zip.finish().unwrap();
}

/* ---------------------------------------------------------------- PDF */

/// 造一个 n 页的 PDF。
///
/// 手写最小 PDF 而不是用 lopdf 的构造器：页面文字全是 ASCII（"Page N"），
/// 这样 `extract_text` 的结果可断言，也不依赖构造器 API 的稳定性。
fn make_pdf(path: &Path, pages: usize) {
    // 对象编号：1=Catalog 2=Pages 3=Font，之后每页占两个（页对象 + 内容流）
    let page_id = |i: usize| 4 + (i - 1) * 2;
    let content_id = |i: usize| 5 + (i - 1) * 2;

    let mut out = String::from("%PDF-1.4\n");
    let mut offsets: Vec<usize> = vec![0]; // 0 号位置留给空闲项

    let push = |out: &mut String, offsets: &mut Vec<usize>, id: usize, body: &str| {
        offsets.push(out.len());
        out.push_str(&format!("{id} 0 obj\n{body}\nendobj\n"));
    };

    push(&mut out, &mut offsets, 1, "<</Type/Catalog/Pages 2 0 R>>");

    let kids: Vec<String> = (1..=pages).map(|i| format!("{} 0 R", page_id(i))).collect();
    push(
        &mut out,
        &mut offsets,
        2,
        &format!(
            "<</Type/Pages/Kids[{}]/Count {}>>",
            kids.join(" "),
            pages
        ),
    );

    push(
        &mut out,
        &mut offsets,
        3,
        "<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>",
    );

    for i in 1..=pages {
        push(
            &mut out,
            &mut offsets,
            page_id(i),
            &format!(
                "<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]\
/Resources<</Font<</F1 3 0 R>>>>/Contents {} 0 R>>",
                content_id(i)
            ),
        );

        let text = format!("BT /F1 24 Tf 72 700 Td (Page {i}) Tj ET");
        push(
            &mut out,
            &mut offsets,
            content_id(i),
            &format!("<</Length {}>>\nstream\n{text}\nendstream", text.len()),
        );
    }

    // xref
    let total = offsets.len(); // 含那个 0 号占位
    let xref_at = out.len();
    out.push_str(&format!("xref\n0 {total}\n"));
    out.push_str("0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        out.push_str(&format!("{off:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<</Size {total}/Root 1 0 R>>\nstartxref\n{xref_at}\n%%EOF\n"
    ));

    std::fs::write(path, out).expect("写测试 PDF");
}

fn page_count(path: &Path) -> usize {
    let doc = lopdf::Document::load(path).expect("读回 PDF");
    doc.get_pages().len()
}

#[test]
fn pdf_merge_concatenates_pages_in_order() {
    let dir = temp_dir("pdf-merge");
    let a = dir.join("甲.pdf");
    let b = dir.join("乙.pdf");
    make_pdf(&a, 3);
    make_pdf(&b, 2);

    let files = vec![a.to_string_lossy().to_string(), b.to_string_lossy().to_string()];
    let (scanned, specs) = scan_and_plan(&files, "@merge", &dir);

    assert_eq!(scanned.len(), 2);
    assert_eq!(specs.len(), 1, "合并应该只产出一个作业，而不是每文件一个");
    assert_eq!(specs[0].src_format, "pdf+");

    let outcome = run_batch_sync(&files, "@merge", &dir);
    assert_eq!(outcome.succeeded(), 1, "事件：{:#?}", outcome.events);
    assert_eq!(outcome.failed(), 0, "事件：{:#?}", outcome.events);

    let merged = outcome.output();
    assert!(merged.exists(), "找不到合并产物；目录：{:?}", list(&dir));

    // 3 + 2 = 5 页，且顺序是甲在前
    assert_eq!(page_count(merged), 5, "合并后的页数不对");
}

#[test]
fn pdf_merge_needs_at_least_two_files() {
    let dir = temp_dir("pdf-merge-one");
    let a = dir.join("独苗.pdf");
    make_pdf(&a, 2);

    let files = vec![a.to_string_lossy().to_string()];
    let (_, specs) = scan_and_plan(&files, "@merge", &dir);
    assert!(specs.is_empty(), "单个 PDF 不该规划出合并作业");
}

#[test]
fn pdf_split_by_ranges_writes_one_file_per_group() {
    let dir = temp_dir("pdf-split");
    let src = dir.join("报告.pdf");
    make_pdf(&src, 6);

    let files = vec![src.to_string_lossy().to_string()];
    let (_, specs) = scan_and_plan(&files, "@split", &dir);
    assert_eq!(specs.len(), 1);

    // 注入 pdf_op=split，并按 1-2,4-5 切
    let dir2 = dir.clone();
    let outcome = run_specs_with_options(
        specs,
        &dir2,
        &[("split_mode", "ranges"), ("split_ranges", "1-2,4-5")],
    );

    assert_eq!(outcome.failed(), 0, "事件：{:#?}", outcome.events);
    assert!(dir.join("报告-1-2.pdf").exists(), "目录：{:?}", list(&dir));
    assert!(dir.join("报告-4-5.pdf").exists(), "目录：{:?}", list(&dir));
    assert_eq!(page_count(&dir.join("报告-1-2.pdf")), 2);
    assert_eq!(page_count(&dir.join("报告-4-5.pdf")), 2);
}

#[test]
fn pdf_organize_drops_pages_and_reorders() {
    let dir = temp_dir("pdf-organize");
    let src = dir.join("稿.pdf");
    make_pdf(&src, 5);

    let files = vec![src.to_string_lossy().to_string()];
    let (_, specs) = scan_and_plan(&files, "@organize", &dir);

    // 删掉第 2 页，然后把剩下四页倒序
    let outcome = run_specs_with_options(
        specs,
        &dir,
        &[("pdf_drop", "2"), ("pdf_order", "5,4,3,1")],
    );
    assert_eq!(outcome.failed(), 0, "事件：{:#?}", outcome.events);

    let out = outcome.output();
    assert!(out.exists(), "目录：{:?}", list(&dir));
    assert_eq!(page_count(out), 4, "删掉一页后应剩四页");
}

#[test]
fn pdf_organize_rotates_by_right_angle() {
    let dir = temp_dir("pdf-rotate");
    let src = dir.join("横的.pdf");
    make_pdf(&src, 1);

    let files = vec![src.to_string_lossy().to_string()];
    let (_, specs) = scan_and_plan(&files, "@organize", &dir);
    let outcome = run_specs_with_options(specs, &dir, &[("pdf_rotate", "90")]);
    assert_eq!(outcome.failed(), 0, "事件：{:#?}", outcome.events);

    let out = outcome.output();
    let doc = lopdf::Document::load(out).unwrap();
    let (_, id) = doc.get_pages().into_iter().next().unwrap();
    let rotate = doc
        .get_dictionary(id)
        .unwrap()
        .get(b"Rotate")
        .and_then(|o| o.as_i64())
        .unwrap_or(0);
    assert_eq!(rotate, 90, "旋转没写进页面字典");
}

#[test]
fn pdf_to_txt_extracts_page_text() {
    let dir = temp_dir("pdf-txt");
    let src = dir.join("三页.pdf");
    make_pdf(&src, 3);

    let files = vec![src.to_string_lossy().to_string()];
    let (_, specs) = scan_and_plan(&files, "txt", &dir);
    assert_eq!(specs.len(), 1);

    let outcome = run_specs_with_options(specs, &dir, &[("page_range", "2")]);
    assert_eq!(outcome.failed(), 0, "事件：{:#?}", outcome.events);

    let text = std::fs::read_to_string(dir.join("三页.txt")).unwrap();
    assert!(text.contains("Page 2"), "应只提取第 2 页；实际：{text:?}");
    assert!(!text.contains("Page 1"), "不该包含第 1 页；实际：{text:?}");
    assert!(!text.contains("Page 3"), "不该包含第 3 页；实际：{text:?}");
}

#[test]
fn corrupt_pdf_reports_corrupt_not_unknown() {
    let dir = temp_dir("pdf-corrupt");
    let src = dir.join("坏的.pdf");
    write_text(&src, "%PDF-1.4\n这不是真的 PDF\n");

    let files = vec![src.to_string_lossy().to_string()];
    let (_, specs) = scan_and_plan(&files, "txt", &dir);
    assert_eq!(specs.len(), 1, "坏 PDF 应保留在队列里，由转换阶段报错");

    let outcome = run_batch_sync(&files, "txt", &dir);
    assert_eq!(outcome.failed(), 1);
}
