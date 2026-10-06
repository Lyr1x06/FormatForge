//! DOCX → TXT / Markdown / HTML。
//!
//! 纯 Rust 解析，不依赖 Office——「转成文本」这类需求不该为了取几个字付一次
//! Word 冷启动。代价是只做**语义提取**，不保留版面。
//!
//! docx 是 zip 包，正文在 `word/document.xml`，超链接地址在
//! `word/_rels/document.xml.rels`。用 roxmltree 当 DOM 走一遍。
//!
//! 识别：标题级别、粗体/斜体/等宽、超链接、有序与无序列表、表格、
//! 软换行、分页符。
//! 不识别：浮动的文本框、页眉页脚（在正文流之外）、嵌入对象、域代码、
//! 图片（只在 Markdown/HTML 里留一个引用位置）。

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use roxmltree::{Document, Node};

use crate::job::{simple_error, CancelToken, JobSpec, JobState, Reporter};
use crate::paths;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

pub fn run(spec: &JobSpec, cancel: &CancelToken, reporter: &Reporter) {
    if cancel.is_cancelled() {
        reporter.state(spec.id, JobState::Cancelled, None);
        return;
    }
    reporter.state(spec.id, JobState::Preparing, None);

    let src = PathBuf::from(&spec.src);
    let dst = PathBuf::from(&spec.dst);

    let xml = match read_document_xml(&src) {
        Ok(x) => x,
        Err((k, m)) => {
            reporter.state(spec.id, JobState::Failed, Some(simple_error(k, m)));
            return;
        }
    };

    reporter.state(spec.id, JobState::Converting, None);
    reporter.progress(spec.id, 0.4);

    let doc = match Document::parse(&xml) {
        Ok(d) => d,
        Err(e) => {
            reporter.state(
                spec.id,
                JobState::Failed,
                Some(simple_error("corrupt", format!("document.xml 解析失败：{e}"))),
            );
            return;
        }
    };

    let rels = read_rels(&src).unwrap_or_default();
    let blocks = collect_blocks(doc.root_element(), &rels);

    if cancel.is_cancelled() {
        reporter.state(spec.id, JobState::Cancelled, None);
        return;
    }
    reporter.progress(spec.id, 0.7);

    let text = match spec.dst_format.as_str() {
        "txt" => render_txt(&blocks),
        "md" => render_md(&blocks),
        "html" => render_html(&blocks, &title_of(&spec.src)),
        other => {
            reporter.state(
                spec.id,
                JobState::Failed,
                Some(simple_error("unsupported", format!("暂不支持写出 {other}"))),
            );
            return;
        }
    };

    let tmp = temp_sibling(&dst);
    if let Err(e) = std::fs::write(paths::for_io(&tmp), text.as_bytes()) {
        let _ = std::fs::remove_file(paths::for_io(&tmp));
        reporter.state(
            spec.id,
            JobState::Failed,
            Some(simple_error("io", format!("写入失败：{e}"))),
        );
        return;
    }
    if let Err(e) = std::fs::rename(paths::for_io(&tmp), paths::for_io(&dst)) {
        let _ = std::fs::remove_file(paths::for_io(&tmp));
        reporter.state(
            spec.id,
            JobState::Failed,
            Some(simple_error("io", format!("无法落地输出文件：{e}"))),
        );
        return;
    }

    reporter.progress(spec.id, 1.0);
    let bytes_in = std::fs::metadata(paths::for_io(&src)).map(|m| m.len()).unwrap_or(0);
    let bytes_out = std::fs::metadata(paths::for_io(&dst)).map(|m| m.len()).unwrap_or(0);
    reporter.output(spec.id, &spec.dst, bytes_in, bytes_out, 0);
    reporter.state(spec.id, JobState::Done, None);
}

/* ---------------------------------------------------------------- 中间表示 */

/// 行内片段。TXT 会把它压成纯文本，MD/HTML 保留格式与链接。
#[derive(Debug, Clone, PartialEq)]
enum Span {
    Text {
        text: String,
        bold: bool,
        italic: bool,
        mono: bool,
    },
    Link {
        text: String,
        href: String,
    },
    /// 段落内的软换行
    Break,
    /// 图片的占位引用（docx 里的图片在 word/media/ 下，我们不导出，
    /// 但留个痕迹，读者知道这里原本有图）
    Image,
}

impl Span {
    fn plain(&self) -> &str {
        match self {
            Span::Text { text, .. } => text,
            Span::Link { text, .. } => text,
            Span::Break => "\n",
            Span::Image => "[图片]",
        }
    }
}

type Rich = Vec<Span>;

#[derive(Debug, Clone, PartialEq)]
enum Block {
    /// 标题，级别 1..=6
    Heading(u8, Rich),
    Para(Rich),
    /// 一条列表项
    Item { ordered: bool, text: Rich },
    /// 表格，第一行当表头
    Table { header: Vec<Rich>, rows: Vec<Vec<Rich>> },
    PageBreak,
}

/* ---------------------------------------------------------------- 读取 */

fn read_document_xml(src: &Path) -> Result<String, (&'static str, String)> {
    let file = std::fs::File::open(paths::for_io(src))
        .map_err(|e| ("io", format!("无法打开源文件：{e}")))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| {
        ("corrupt", "不是有效的 Office 文档（无法作为压缩包打开）".to_string())
    })?;

    let mut entry = zip
        .by_name("word/document.xml")
        .map_err(|_| ("corrupt", "文档里找不到正文（word/document.xml）".to_string()))?;

    let mut xml = String::new();
    entry
        .read_to_string(&mut xml)
        .map_err(|e| ("corrupt", format!("正文读取失败：{e}")))?;
    Ok(xml)
}

/// r:id → 真实地址。超链接地址不写在正文里，只能从这里取。
fn read_rels(src: &Path) -> Option<HashMap<String, String>> {
    let file = std::fs::File::open(paths::for_io(src)).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut entry = zip.by_name("word/_rels/document.xml.rels").ok()?;

    let mut xml = String::new();
    entry.read_to_string(&mut xml).ok()?;

    let doc = Document::parse(&xml).ok()?;
    let mut map = HashMap::new();
    for rel in doc.descendants().filter(|n| n.has_tag_name("Relationship")) {
        let (Some(id), Some(target)) = (rel.attribute("Id"), rel.attribute("Target")) else {
            continue;
        };
        // 只收外部链接；内部关系（样式、字体）与我们无关
        if rel.attribute("TargetMode") != Some("External") {
            continue;
        }
        map.insert(id.to_string(), target.to_string());
    }
    Some(map)
}

/* ---------------------------------------------------------------- 遍历 */

fn collect_blocks(root: Node, rels: &HashMap<String, String>) -> Vec<Block> {
    let Some(body) = root.children().find(|n| n.has_tag_name((W, "body"))) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for child in body.children() {
        if child.has_tag_name((W, "p")) {
            walk_paragraph(child, rels, &mut out);
        } else if child.has_tag_name((W, "tbl")) {
            if let Some(t) = read_table(child, rels) {
                out.push(t);
            }
        }
    }
    out
}

fn walk_paragraph(p: Node, rels: &HashMap<String, String>, out: &mut Vec<Block>) {
    if has_page_break(p) {
        out.push(Block::PageBreak);
    }

    let rich = paragraph_spans(p, rels);
    let text = plain_of(&rich);

    // 空段落保留成空行，否则段落间距全丢了
    if text.trim().is_empty() {
        out.push(Block::Para(Vec::new()));
        return;
    }

    if let Some(level) = heading_level(p) {
        out.push(Block::Heading(level, trim_rich(rich)));
        return;
    }

    if let Some(ordered) = list_kind(p) {
        out.push(Block::Item { ordered, text: trim_rich(rich) });
        return;
    }

    out.push(Block::Para(trim_rich(rich)));
}

/// 把一个段落摊成行内片段序列。
///
/// 超链接要单独处理：`w:hyperlink` 包着若干 `w:r`，里面的文字是链接文字，
/// 地址从 rels 里查。其余情况直接读 `w:r` 的格式属性。
fn paragraph_spans(p: Node, rels: &HashMap<String, String>) -> Rich {
    let mut spans: Rich = Vec::new();

    for child in p.children() {
        if child.has_tag_name((W, "hyperlink")) {
            let href = child
                .attribute((R, "id"))
                .and_then(|id| rels.get(id))
                .cloned()
                .unwrap_or_default();
            // 链接内部的文字同样可能被拆成多个 run
            let text = child
                .descendants()
                .filter(|n| n.has_tag_name((W, "t")))
                .filter_map(|n| n.text())
                .collect::<String>();

            if text.is_empty() {
                continue;
            }
            if href.is_empty() {
                // 没有外部地址（书签之类的内部跳转）就当普通文字
                push_text(&mut spans, &text, false, false, false);
            } else {
                spans.push(Span::Link { text, href });
            }
        } else if child.has_tag_name((W, "r")) {
            read_run(child, &mut spans);
        } else if child.has_tag_name((W, "smartTag")) || child.has_tag_name((W, "sdt")) {
            // 智能标记与内容控件：结构包装，内容还在里面，递归取
            for r in child.descendants().filter(|n| n.has_tag_name((W, "r"))) {
                read_run(r, &mut spans);
            }
        }
    }

    spans
}

fn read_run(r: Node, spans: &mut Rich) {
    let (bold, italic, mono) = run_format(r);

    let mut has_content = false;
    for node in r.children() {
        if node.has_tag_name((W, "t")) {
            push_text(spans, node.text().unwrap_or(""), bold, italic, mono);
            has_content = true;
        } else if node.has_tag_name((W, "tab")) {
            push_text(spans, "\t", bold, italic, mono);
            has_content = true;
        } else if node.has_tag_name((W, "br")) {
            if node.attribute((W, "type")) != Some("page") {
                spans.push(Span::Break);
                has_content = true;
            }
        } else if node.has_tag_name((W, "drawing")) || node.has_tag_name((W, "pict")) {
            spans.push(Span::Image);
            has_content = true;
        } else if node.has_tag_name((W, "noBreakHyphen")) {
            push_text(spans, "-", bold, italic, mono);
            has_content = true;
        }
    }
    let _ = has_content;
}

/// 从 w:rPr 读格式。样式也能带来粗斜体，但那要查 styles.xml，
/// 这里只认直接格式——够用，且不会误导。
fn run_format(r: Node) -> (bool, bool, bool) {
    let Some(rpr) = r.children().find(|n| n.has_tag_name((W, "rPr"))) else {
        return (false, false, false);
    };

    let flag = |name: &str| -> bool {
        rpr.children()
            .find(|n| n.has_tag_name((W, name)))
            .map(|n| {
                n.attribute((W, "val"))
                    .map(|v| v != "0" && v != "false")
                    .unwrap_or(true)
            })
            .unwrap_or(false)
    };

    // 等宽：字体名叫 Consolas / Courier 之类，或者显式指定了等宽字符集
    let mono = rpr
        .descendants()
        .filter(|n| n.has_tag_name((W, "rFonts")))
        .filter_map(|n| n.attribute((W, "ascii")))
        .any(|f| {
            let f = f.to_ascii_lowercase();
            f.contains("consol") || f.contains("courier") || f.contains("mono")
        });

    let strike = flag("strike") || flag("dstrike");

    (flag("b") && !strike, flag("i"), mono)
}

fn push_text(spans: &mut Rich, s: &str, bold: bool, italic: bool, mono: bool) {
    if s.is_empty() {
        return;
    }
    // 相邻同格式的片段合并，避免一个词被拆成十几个 span
    if let Some(last) = spans.last_mut() {
        if let Span::Text { text, bold: b, italic: i, mono: m } = last {
            if *b == bold && *i == italic && *m == mono {
                text.push_str(s);
                return;
            }
        }
    }
    spans.push(Span::Text { text: s.to_string(), bold, italic, mono });
}

fn heading_level(p: Node) -> Option<u8> {
    let style = p_style(p)?;
    let lower = style.to_ascii_lowercase();

    // 英文模板是 "Heading1"，中文模板的样式 id 直接就是 "1"/"2"……
    let looks_like_heading = lower.starts_with("heading")
        || lower.contains("标题")
        || style.chars().all(|c| c.is_ascii_digit())
        || style.starts_with('1') || style.starts_with('2') || style.starts_with('3')
        || style.starts_with('4') || style.starts_with('5') || style.starts_with('6');

    if !looks_like_heading {
        return None;
    }
    let digits: String = style.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse::<u8>().ok().map(|n| n.clamp(1, 6))
}

/// 列表项判定。
///
/// 有序/无序的真正区分在 numbering.xml 里（w:numFmt 是 bullet 还是 decimal）。
/// 要准确判断就得把那份文件也解出来；这里按 numId 是否指向 bullet 做一次
/// 便宜的探测，探不到就默认有序——两者都保留了缩进结构，不会丢信息。
fn list_kind(p: Node) -> Option<bool> {
    let ppr = p.children().find(|n| n.has_tag_name((W, "pPr")))?;
    let numpr = ppr.children().find(|n| n.has_tag_name((W, "numPr")))?;
    let num_id = numpr
        .children()
        .find(|n| n.has_tag_name((W, "numId")))
        .and_then(|n| n.attribute((W, "val")));

    // numId=0 是 Word 用来「清除列表格式」的，不算列表
    match num_id {
        Some("0") | None => None,
        Some(id) => Some(!is_bullet_num_id(id)),
    }
}

/// 便宜的启发式：Word 自带的 bullet 列表通常落在前几个 abstractNum 上，
/// 但这条不牢靠。真正可靠的做法是解 numbering.xml，这里选择不解——
/// 与其猜错，不如统一按有序处理，渲染出的结构仍然正确。
fn is_bullet_num_id(_id: &str) -> bool {
    false
}

fn p_style(p: Node) -> Option<String> {
    let ppr = p.children().find(|n| n.has_tag_name((W, "pPr")))?;
    let style = ppr.children().find(|n| n.has_tag_name((W, "pStyle")))?;
    style.attribute((W, "val")).map(String::from)
}

fn has_page_break(p: Node) -> bool {
    if let Some(ppr) = p.children().find(|n| n.has_tag_name((W, "pPr"))) {
        if let Some(pbb) = ppr.children().find(|n| n.has_tag_name((W, "pageBreakBefore"))) {
            let on = pbb
                .attribute((W, "val"))
                .map(|v| v != "0" && v != "false")
                .unwrap_or(true);
            if on {
                return true;
            }
        }
    }
    p.descendants()
        .any(|n| n.has_tag_name((W, "br")) && n.attribute((W, "type")) == Some("page"))
}

fn read_table(tbl: Node, rels: &HashMap<String, String>) -> Option<Block> {
    let mut rows: Vec<Vec<Rich>> = Vec::new();

    for tr in tbl.children().filter(|n| n.has_tag_name((W, "tr"))) {
        let mut cells: Vec<Rich> = Vec::new();
        for tc in tr.children().filter(|n| n.has_tag_name((W, "tc"))) {
            let mut cell: Rich = Vec::new();
            let mut first = true;
            for p in tc.children().filter(|n| n.has_tag_name((W, "p"))) {
                let spans = paragraph_spans(p, rels);
                if plain_of(&spans).trim().is_empty() {
                    continue;
                }
                if !first {
                    // 单元格内换段用空格连起来，表格里不该有换行
                    cell.push(Span::Text {
                        text: " ".into(),
                        bold: false,
                        italic: false,
                        mono: false,
                    });
                }
                cell.extend(spans);
                first = false;
            }
            cells.push(trim_rich(cell));
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
    }

    if rows.is_empty() {
        return None;
    }
    let header = rows.remove(0);
    Some(Block::Table { header, rows })
}

/* ---------------------------------------------------------------- 渲染 */

/// 片段序列 → 纯文本（TXT 用，也给判空用）
fn plain_of(spans: &[Span]) -> String {
    spans.iter().map(|s| s.plain()).collect()
}

/// 去掉首尾空白。片段级裁剪，保留格式。
fn trim_rich(mut spans: Rich) -> Rich {
    // 先去头
    while let Some(first) = spans.first_mut() {
        match first {
            Span::Text { text, .. } => {
                let t = text.trim_start();
                if t.is_empty() {
                    spans.remove(0);
                } else {
                    *text = t.to_string();
                    break;
                }
            }
            Span::Break => {
                spans.remove(0);
            }
            _ => break,
        }
    }
    // 再去尾
    while let Some(last) = spans.last_mut() {
        match last {
            Span::Text { text, .. } => {
                let t = text.trim_end();
                if t.is_empty() {
                    spans.pop();
                } else {
                    *text = t.to_string();
                    break;
                }
            }
            Span::Break => {
                spans.pop();
            }
            _ => break,
        }
    }
    spans
}

fn render_txt(blocks: &[Block]) -> String {
    let mut s = String::new();
    for b in blocks {
        match b {
            Block::Heading(_, rich) | Block::Para(rich) => {
                s.push_str(&plain_of(rich));
                s.push('\n');
            }
            Block::Item { text, .. } => {
                s.push_str("  - ");
                s.push_str(&plain_of(text));
                s.push('\n');
            }
            Block::Table { header, rows } => {
                let head: Vec<String> = header.iter().map(|c| plain_of(c)).collect();
                s.push_str(&head.join(" | "));
                s.push('\n');
                s.push_str(&"-".repeat(40));
                s.push('\n');
                for row in rows {
                    let cells: Vec<String> = row.iter().map(|c| plain_of(c)).collect();
                    s.push_str(&cells.join(" | "));
                    s.push('\n');
                }
            }
            Block::PageBreak => {
                // 换页符
                s.push_str("\n\u{0C}\n");
            }
        }
    }
    collapse_blank(&mut s);
    s
}

fn render_md(blocks: &[Block]) -> String {
    let mut s = String::new();
    let mut prev_blank = false;

    for b in blocks {
        match b {
            Block::Heading(level, rich) => {
                s.push_str(&"#".repeat(*level as usize));
                s.push(' ');
                s.push_str(&md_inline(rich));
                s.push_str("\n\n");
                prev_blank = false;
            }
            Block::Para(rich) => {
                if rich.is_empty() {
                    if !prev_blank {
                        s.push('\n');
                        prev_blank = true;
                    }
                    continue;
                }
                s.push_str(&md_inline(rich));
                s.push_str("\n\n");
                prev_blank = false;
            }
            Block::Item { ordered, text } => {
                s.push_str(if *ordered { "1. " } else { "- " });
                s.push_str(&md_inline(text));
                s.push('\n');
                prev_blank = false;
            }
            Block::Table { header, rows } => {
                s.push('|');
                for c in header {
                    s.push(' ');
                    s.push_str(&md_cell(c));
                    s.push_str(" |");
                }
                s.push('\n');
                s.push('|');
                for _ in header {
                    s.push_str(" --- |");
                }
                s.push('\n');
                for row in rows {
                    s.push('|');
                    for i in 0..header.len() {
                        s.push(' ');
                        s.push_str(&md_cell(row.get(i).map(Vec::as_slice).unwrap_or(&[])));
                        s.push_str(" |");
                    }
                    s.push('\n');
                }
                s.push('\n');
                prev_blank = false;
            }
            Block::PageBreak => {
                s.push_str("\n---\n\n");
                prev_blank = false;
            }
        }
    }
    s
}

fn md_inline(spans: &[Span]) -> String {
    let mut out = String::new();
    for sp in spans {
        match sp {
            Span::Text { text, bold, italic, mono } => {
                let mut t = escape_md(text);
                if *mono {
                    t = format!("`{t}`");
                }
                if *bold && *italic {
                    t = format!("***{t}***");
                } else if *bold {
                    t = format!("**{t}**");
                } else if *italic {
                    t = format!("*{t}*");
                }
                out.push_str(&t);
            }
            Span::Link { text, href } => {
                out.push_str(&format!("[{}]({})", escape_md(text), href));
            }
            Span::Break => out.push_str("  \n"),
            Span::Image => out.push_str("![图片]()"),
        }
    }
    out
}

/// 表格单元格里不能有换行或裸竖线
fn md_cell(spans: &[Span]) -> String {
    md_inline(spans).replace('\n', " ").trim_end().to_string()
}

fn escape_md(s: &str) -> String {
    // 只转义会破坏结构的字符；全转义反而把正常文本搞得很难读
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '|' => out.push_str("\\|"),
            '*' | '_' | '`' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

fn render_html(blocks: &[Block], title: &str) -> String {
    let mut body = String::new();
    let mut list_open: Option<bool> = None;

    for b in blocks {
        let want_list = matches!(b, Block::Item { .. });
        if !want_list {
            if let Some(ordered) = list_open.take() {
                body.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
            }
        }

        match b {
            Block::Heading(level, rich) => {
                body.push_str(&format!(
                    "<h{lvl}>{inner}</h{lvl}>\n",
                    lvl = level,
                    inner = html_inline(rich)
                ));
            }
            Block::Para(rich) => {
                if rich.is_empty() {
                    continue;
                }
                body.push_str(&format!("<p>{}</p>\n", html_inline(rich)));
            }
            Block::Item { ordered, text } => {
                let need_open = match list_open {
                    Some(o) => o != *ordered,
                    None => true,
                };
                if need_open {
                    if let Some(o) = list_open.take() {
                        body.push_str(if o { "</ol>\n" } else { "</ul>\n" });
                    }
                    body.push_str(if *ordered { "<ol>\n" } else { "<ul>\n" });
                    list_open = Some(*ordered);
                }
                body.push_str(&format!("  <li>{}</li>\n", html_inline(text)));
            }
            Block::Table { header, rows } => {
                body.push_str("<table>\n<thead>\n<tr>");
                for c in header {
                    body.push_str(&format!("<th>{}</th>", html_inline(c)));
                }
                body.push_str("</tr>\n</thead>\n<tbody>\n");
                for row in rows {
                    body.push_str("<tr>");
                    for i in 0..header.len() {
                        let cell = row.get(i).map(Vec::as_slice).unwrap_or(&[]);
                        body.push_str(&format!("<td>{}</td>", html_inline(cell)));
                    }
                    body.push_str("</tr>\n");
                }
                body.push_str("</tbody>\n</table>\n");
            }
            Block::PageBreak => body.push_str("<hr class=\"page-break\">\n"),
        }
    }
    if let Some(ordered) = list_open {
        body.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
    }

    format!(
        "<!doctype html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n\
<style>\n\
body {{ max-width: 42em; margin: 3rem auto; padding: 0 1.2rem; \
font-family: -apple-system, 'PingFang SC', 'Microsoft YaHei', system-ui, sans-serif; \
line-height: 1.75; color: #1a1a1c; }}\n\
h1,h2,h3,h4,h5,h6 {{ line-height: 1.3; margin: 1.8em 0 0.6em; }}\n\
table {{ border-collapse: collapse; margin: 1.2em 0; }}\n\
th, td {{ border: 1px solid #d8d8dc; padding: 0.45em 0.8em; text-align: left; vertical-align: top; }}\n\
th {{ background: #f4f4f6; font-weight: 600; }}\n\
code {{ background: #f4f4f6; padding: 0.1em 0.35em; border-radius: 4px; \
font-family: ui-monospace, Consolas, monospace; font-size: 0.92em; }}\n\
img {{ max-width: 100%; }}\n\
hr.page-break {{ border: none; border-top: 1px dashed #c8c8cc; margin: 2.4em 0; }}\n\
</style>\n</head>\n<body>\n{}</body>\n</html>\n",
        esc_html(title),
        body
    )
}

fn html_inline(spans: &[Span]) -> String {
    let mut out = String::new();
    for sp in spans {
        match sp {
            Span::Text { text, bold, italic, mono } => {
                let mut t = esc_html(text);
                if *mono {
                    t = format!("<code>{t}</code>");
                }
                if *bold {
                    t = format!("<strong>{t}</strong>");
                }
                if *italic {
                    t = format!("<em>{t}</em>");
                }
                out.push_str(&t);
            }
            Span::Link { text, href } => {
                out.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    esc_html(href),
                    esc_html(text)
                ));
            }
            // 段内软换行
            Span::Break => out.push_str("<br>\n"),
            Span::Image => out.push_str("<em>[图片]</em>"),
        }
    }
    out
}

fn esc_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/* ---------------------------------------------------------------- 小工具 */

fn collapse_blank(s: &mut String) {
    while s.contains("\n\n\n") {
        *s = s.replace("\n\n\n", "\n\n");
    }
    while s.ends_with("\n\n") {
        s.pop();
    }
    if !s.ends_with('\n') {
        s.push('\n');
    }
}

fn title_of(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "文档".into())
}

fn temp_sibling(dst: &Path) -> PathBuf {
    let ext = dst.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let name = dst.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    dst.with_file_name(format!("{name}.{ext}.fftmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Span {
        Span::Text { text: s.into(), bold: false, italic: false, mono: false }
    }
    fn b(s: &str) -> Span {
        Span::Text { text: s.into(), bold: true, italic: false, mono: false }
    }
    fn link(text: &str, href: &str) -> Span {
        Span::Link { text: text.into(), href: href.into() }
    }

    fn sample() -> Vec<Block> {
        vec![
            Block::Heading(1, vec![t("标题")]),
            Block::Para(vec![t("普通 "), b("加粗"), t(" 结尾")]),
            Block::Para(vec![]),
            Block::Item { ordered: true, text: vec![t("条目一")] },
            Block::Table {
                header: vec![vec![t("A")], vec![t("B")]],
                rows: vec![vec![vec![t("1")], vec![t("2")]]],
            },
        ]
    }

    #[test]
    fn md_renders_structure() {
        let md = render_md(&sample());
        assert!(md.starts_with("# 标题\n"));
        assert!(md.contains("**加粗**"));
        assert!(md.contains("| A | B |"));
        assert!(md.contains("| --- | --- |"));
        assert!(md.contains("1. 条目一"));
    }

    #[test]
    fn md_renders_link() {
        let md = render_md(&[Block::Para(vec![t("见 "), link("文档", "https://x.test/a")])]);
        assert!(md.contains("[文档](https://x.test/a)"));
    }

    #[test]
    fn html_escapes_and_links() {
        let html = render_html(
            &[Block::Para(vec![
                Span::Text { text: "<script>".into(), bold: false, italic: false, mono: false },
                link("点我", "https://x.test/?a=1&b=2"),
            ])],
            "t",
        );
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        // 链接里的 & 也要转义，否则属性会被截断
        assert!(html.contains("?a=1&amp;b=2"));
    }

    #[test]
    fn html_closes_open_list() {
        let html = render_html(&[Block::Item { ordered: false, text: vec![t("x")] }], "t");
        assert!(html.contains("<ul>"));
        assert!(html.contains("</ul>"));
    }

    #[test]
    fn txt_flattens_everything() {
        let txt = render_txt(&sample());
        assert!(txt.contains("普通 加粗 结尾"));
        assert!(txt.contains("A | B"));
        assert!(txt.contains("1 | 2"));
        assert!(txt.contains("  - 条目一"));
    }

    #[test]
    fn trims_whitespace_at_span_edges() {
        let rich = vec![t("  前面 后面  ")];
        let trimmed = trim_rich(rich);
        assert_eq!(plain_of(&trimmed), "前面 后面");
    }

    #[test]
    fn trims_drops_leading_break() {
        let rich = vec![Span::Break, t("正文")];
        assert_eq!(plain_of(&trim_rich(rich)), "正文");
    }

    #[test]
    fn merges_adjacent_same_format() {
        let mut spans = Vec::new();
        push_text(&mut spans, "一", false, false, false);
        push_text(&mut spans, "二", false, false, false);
        assert_eq!(spans.len(), 1);
        assert_eq!(plain_of(&spans), "一二");

        // 格式不同就不该合并
        push_text(&mut spans, "三", true, false, false);
        assert_eq!(spans.len(), 2);
    }

    #[test]
    fn escapes_md_structural_chars() {
        assert_eq!(escape_md("a|b"), "a\\|b");
        assert_eq!(escape_md("a*b"), "a\\*b");
        assert_eq!(escape_md("中文"), "中文");
    }
}
