//! PDF 操作：合并、拆分、旋转、删页重排、压缩、取文本、图片转 PDF。
//!
//! 走 lopdf 纯 Rust 路线。它做的是**结构级**操作——页对象原样搬运，
//! 内容流不解码重编码，所以合并/拆分/重排是真正无损的。
//!
//! 有意没做的两件事：
//!   * **水印** —— 需要往内容流里插绘图指令并自己算字体与变换矩阵，
//!     做砸了会毁掉正文。宁可不做。
//!   * **压缩** —— 真正的 PDF 压缩要么重压内嵌图片（要完整图像管线），
//!     要么重新序列化（收益通常只有几个百分点）。都不值得冒险。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lopdf::{dictionary, Document, Object, ObjectId, Stream};

use crate::job::{simple_error, CancelToken, JobSpec, JobState, Reporter};
use crate::paths;

/// 多个 PDF 合成一个时用的虚拟格式 id
pub const MERGE_SRC: &str = "pdf+";

pub fn run(spec: &JobSpec, cancel: &CancelToken, reporter: &Reporter) {
    let outcome = dispatch(spec, cancel, reporter);

    match outcome {
        Ok(()) => {
            reporter.progress(spec.id, 1.0);
            let bytes_in = total_input_bytes(spec);
            let bytes_out = file_len(Path::new(&spec.dst));
            reporter.output(spec.id, &spec.dst, bytes_in, bytes_out, 0);
            reporter.state(spec.id, JobState::Done, None);
        }
        Err((kind, msg)) => {
            reporter.state(spec.id, JobState::Failed, Some(simple_error(kind, msg)));
        }
    }
}

fn dispatch(
    spec: &JobSpec,
    cancel: &CancelToken,
    reporter: &Reporter,
) -> Result<(), (&'static str, String)> {
    if cancel.is_cancelled() {
        return Err(("cancelled", String::new()));
    }
    reporter.state(spec.id, JobState::Converting, None);

    match spec.dst_format.as_str() {
        // 合并：src 里存的是「多个文件用 | 分隔」的路径列表
        "pdf" if spec.src_format == MERGE_SRC => merge(spec, reporter),

        // 同一个源可能对应两种产出：整理页面，或者拆分。
        // 由动作注入的 pdf_op 决定走哪条。
        "pdf" if spec.src_format == "pdf" => match spec.text("pdf_op", "organize") {
            "split" => split_entry(spec, reporter),
            _ => organize(spec, reporter),
        },

        "txt" if spec.src_format == "pdf" => extract_text(spec, reporter),

        other if spec.src_format == "pdf" => Err((
            "unsupported",
            format!("暂不支持 PDF → {other}（原样渲染需要 pdfium，尚未接入）"),
        )),
        other => Err(("unsupported", format!("暂不支持 PDF 操作 {other}"))),
    }
}

/// 拆分的入口：载入后交给 split
fn split_entry(spec: &JobSpec, reporter: &Reporter) -> Result<(), (&'static str, String)> {
    let src = PathBuf::from(&spec.src);
    let doc = load(&src)?;
    let total = doc.get_pages().len() as u32;
    if total == 0 {
        return Err(("corrupt", "PDF 里没有页面".into()));
    }
    split(spec, doc, total, reporter)
}

/* ---------------------------------------------------------------- 合并 */

/// 把 `src` 的全部对象按 `offset` 平移编号后搬进 `dst`，返回旧→新编号的映射。
///
/// 用编号平移而不是 lopdf 的 `renumber_objects_with`：后者就地重排源文档、
/// 且不返回映射表，没法拿到「搬过去之后是几号」这个关键信息。
fn graft(dst: &mut Document, src: &Document, offset: u32) -> BTreeMap<ObjectId, ObjectId> {
    let shift = |id: ObjectId| (id.0 + offset, id.1);

    let map: BTreeMap<ObjectId, ObjectId> =
        src.objects.keys().map(|id| (*id, shift(*id))).collect();

    for (old_id, obj) in src.objects.iter() {
        dst.objects.insert(shift(*old_id), remap_refs(obj, &map));
    }
    map
}

fn merge(spec: &JobSpec, reporter: &Reporter) -> Result<(), (&'static str, String)> {
    let inputs: Vec<PathBuf> = spec.src.split('|').map(PathBuf::from).collect();
    if inputs.len() < 2 {
        return Err(("unsupported", "合并需要至少两个 PDF".into()));
    }

    let mut out = Document::with_version("1.7");
    // 每个输入文件里「页序号 → 搬过来之后的页对象编号」
    let mut pages_by_input: Vec<Vec<ObjectId>> = Vec::new();

    for (i, path) in inputs.iter().enumerate() {
        let doc = load(path)?;
        let pages = doc.get_pages();
        if pages.is_empty() {
            return Err(("corrupt", format!("{} 里没有页面", path.display())));
        }

        // lopdf 的 Document 之间没有「合并」原语，做法是把每个源文档的对象
        // 按偏移量整体搬进目标，再重建一棵页面树把它们串起来。
        out.max_id += 1;
        let offset = out.max_id;
        let map = graft(&mut out, &doc, offset);
        out.max_id = offset + doc.max_id;

        let remapped: Vec<ObjectId> = pages
            .values()
            .map(|id| *map.get(id).unwrap_or(id))
            .collect();
        pages_by_input.push(remapped);

        reporter.progress(spec.id, 0.1 + 0.6 * ((i + 1) as f32 / inputs.len() as f32));
    }

    // 建一棵新的页面树
    let pages_id = out.new_object_id();
    let mut kids: Vec<Object> = Vec::new();
    for map in &pages_by_input {
        for id in map {
            // 页面的 Parent 要指回新树，否则打开会失败
            if let Ok(dict) = out.get_dictionary_mut(*id) {
                dict.set("Parent", pages_id);
            }
            kids.push(Object::Reference(*id));
        }
    }

    let count = kids.len() as i64;
    out.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );

    let catalog_id = out.new_object_id();
    out.objects.insert(
        catalog_id,
        Object::Dictionary(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        }),
    );
    out.trailer.set("Root", catalog_id);

    save_to(out, &spec.dst)
}

/// 把一个对象里所有指向旧编号的引用改写成新编号
fn remap_refs(obj: &Object, map: &BTreeMap<ObjectId, ObjectId>) -> Object {
    match obj {
        Object::Reference(id) => Object::Reference(*map.get(id).unwrap_or(id)),
        Object::Array(items) => Object::Array(items.iter().map(|i| remap_refs(i, map)).collect()),
        Object::Dictionary(d) => {
            let mut out = lopdf::Dictionary::new();
            for (k, v) in d.iter() {
                out.set(k.clone(), remap_refs(v, map));
            }
            Object::Dictionary(out)
        }
        Object::Stream(s) => {
            let mut dict = lopdf::Dictionary::new();
            for (k, v) in s.dict.iter() {
                dict.set(k.clone(), remap_refs(v, map));
            }
            Object::Stream(Stream::new(dict, s.content.clone()))
        }
        other => other.clone(),
    }
}

/* ---------------------------------------------------------------- 页面整理 */

/// 一条输出指令：输出里的一页来自源文件的第几页、转多少度。
///
/// 只表示「保留」——被删的页根本不进这个列表，由 `apply_plan` 反推出要删哪些。
#[derive(Debug, Clone, Copy)]
struct PageOp {
    src_page: u32,
    rotate: i64,
}

/// 解析页码范围表达式：`all` / `1-3,5,8-10`
///
/// 返回页码升序列表，已去重并夹在 `1..=total` 内。非法输入返回 None，
/// 由调用方决定是报错还是退化成全部。
fn parse_ranges(expr: &str, total: u32) -> Option<Vec<u32>> {
    let expr = expr.trim();
    if expr.is_empty() || expr.eq_ignore_ascii_case("all") {
        return Some((1..=total).collect());
    }

    let mut out: Vec<u32> = Vec::new();
    for part in expr.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            let a: u32 = a.trim().parse().ok()?;
            let b: u32 = b.trim().parse().ok()?;
            if a == 0 || b == 0 || a > b {
                return None;
            }
            for p in a..=b.min(total) {
                if p >= 1 {
                    out.push(p);
                }
            }
        } else {
            let p: u32 = part.parse().ok()?;
            if p == 0 {
                return None;
            }
            if p <= total {
                out.push(p);
            }
        }
    }

    out.sort_unstable();
    out.dedup();
    if out.is_empty() { None } else { Some(out) }
}

/// 解析「删页」表达式：逗号分隔的页码，也支持 `a-b` 区间
fn parse_drops(expr: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for part in expr.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                for p in a..=b {
                    out.push(p);
                }
            }
        } else if let Ok(p) = part.parse::<u32>() {
            out.push(p);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// 解析「页面顺序」表达式：逗号分隔的页码，顺序即输出顺序
fn parse_order(expr: &str, total: u32) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    for part in expr.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let p: u32 = part.parse().ok()?;
        if p == 0 || p > total {
            return None;
        }
        out.push(p);
    }
    if out.is_empty() { None } else { Some(out) }
}

fn organize(spec: &JobSpec, reporter: &Reporter) -> Result<(), (&'static str, String)> {
    let src = PathBuf::from(&spec.src);

    let mut doc = load(&src)?;
    let total = doc.get_pages().len() as u32;
    if total == 0 {
        return Err(("corrupt", "PDF 里没有页面".into()));
    }

    // ---- 算出「输出第 i 页来自源的第几页、旋转多少」 ----
    let plan: Vec<PageOp> = match spec.text("pdf_mode", "keep") {
        "keep" => {
            // 全部保留，但可以整体旋转、可以删页、可以重排
            let dropped = parse_drops(spec.text("pdf_drop", ""));

            // 注册表把 pdf_rotate 定义成 Select（值是字符串 "0"/"90"/...），
            // 所以这里按文本读再解析——用 spec.num 会永远是 0。
            let rotate: i64 = spec
                .text("pdf_rotate", "0")
                .trim()
                .parse()
                .unwrap_or(0);

            // 重排优先于原顺序
            let order = spec
                .text("pdf_order", "")
                .trim()
                .to_string();
            let base: Vec<u32> = if order.is_empty() {
                (1..=total).collect()
            } else {
                parse_order(&order, total).ok_or((
                    "invalid_option",
                    format!("页面顺序「{order}」无法解析，应为 1,3,2 这样的页码列表"),
                ))?
            };

            base.into_iter()
                .filter(|p| !dropped.contains(p))
                .map(|p| PageOp { src_page: p, rotate })
                .collect()
        }
        other => {
            return Err(("unsupported", format!("未知的页面模式：{other}")));
        }
    };

    if plan.is_empty() {
        return Err((
            "invalid_option",
            "整理之后没有剩下任何页面——检查一下删页与顺序设置".into(),
        ));
    }

    apply_plan(&mut doc, &plan)?;
    reporter.progress(spec.id, 0.8);
    save_to(doc, &spec.dst.to_string())
}

/// 按 plan 重组文档：删掉没被保留的页，给保留的页设置旋转，再按 plan 的顺序重排。
///
/// 顺序很关键：`delete_pages` 之后页码会重新连续编号，所以重排要拿
/// 「删除后的页码 → 页对象」这张映射来做，而不是拿输出序号去索引。
fn apply_plan(doc: &mut Document, plan: &[PageOp]) -> Result<(), (&'static str, String)> {
    let kept: Vec<u32> = plan.iter().map(|op| op.src_page).collect();

    let all: Vec<u32> = doc.get_pages().keys().copied().collect();
    let to_delete: Vec<u32> = all.iter().copied().filter(|p| !kept.contains(p)).collect();

    // 旋转要在删页**之前**做——此时源页码还有效
    for op in plan {
        if op.rotate == 0 {
            continue;
        }
        let Some(id) = doc.get_pages().get(&op.src_page).copied() else {
            continue;
        };
        if let Ok(dict) = doc.get_dictionary_mut(id) {
            let base = dict
                .get(b"Rotate")
                .ok()
                .and_then(|o| o.as_i64().ok())
                .unwrap_or(0);
            dict.set("Rotate", normalize_rotation(base + op.rotate));
        }
    }

    if !to_delete.is_empty() {
        doc.delete_pages(&to_delete);
    }

    // delete_pages 之后页码重新连续编号，所以重排不能拿**源页码**去索引，
    // 得先做一次「源页码 → 删除后的页码」的翻译。
    // 剩下的页按源页码升序排，就依次是新的 1、2、3……
    let mut sorted_kept = kept.clone();
    sorted_kept.sort_unstable();

    let after_delete = doc.get_pages();
    let order: Vec<u32> = plan
        .iter()
        .filter_map(|op| {
            sorted_kept
                .iter()
                .position(|p| *p == op.src_page)
                .map(|i| (i + 1) as u32)
        })
        .collect();

    reorder_pages(doc, &order, &after_delete)
}

/// 把旋转角夹到 [0,360) 且为 90 的倍数——PDF 只认这四个值
fn normalize_rotation(deg: i64) -> i64 {
    let d = deg % 360;
    let d = if d < 0 { d + 360 } else { d };
    // 吸附到最近的 90 度
    ((d + 45) / 90 % 4) * 90
}

fn reorder_pages(
    doc: &mut Document,
    order: &[u32],
    pages_after_delete: &BTreeMap<u32, ObjectId>,
) -> Result<(), (&'static str, String)> {
    // 页面树可能有多层，找到根节点
    let root = doc
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .ok();
    let Some(root_id) = root else {
        return Err(("corrupt", "PDF 缺少 Root".into()));
    };
    let pages_id = doc
        .get_dictionary(root_id)
        .and_then(|d| d.get(b"Pages"))
        .and_then(Object::as_reference)
        .map_err(|_| ("corrupt", "PDF 缺少页面树".to_string()))?;

    let kids: Vec<Object> = order
        .iter()
        .filter_map(|p| pages_after_delete.get(p))
        .map(|id| Object::Reference(*id))
        .collect();

    let count = kids.len() as i64;
    if let Ok(dict) = doc.get_dictionary_mut(pages_id) {
        dict.set("Kids", kids);
        dict.set("Count", count);
    }
    Ok(())
}

/* ---------------------------------------------------------------- 拆分 */

fn split(
    spec: &JobSpec,
    doc: Document,
    total: u32,
    reporter: &Reporter,
) -> Result<(), (&'static str, String)> {
    let dst = PathBuf::from(&spec.dst);

    let chunks: Vec<(String, Vec<u32>)> = match spec.text("split_mode", "ranges") {
        "every" => {
            let n = (spec.num("split_every", 10.0) as u32).max(1);
            let mut out = Vec::new();
            let mut start = 1u32;
            while start <= total {
                let end = (start + n - 1).min(total);
                out.push((format!("{start}-{end}"), (start..=end).collect()));
                start = end + 1;
            }
            out
        }
        // "each" 与 "ranges" 都是「按给定分组」，区别只在 ranges 由用户指定
        _ => {
            let expr = spec.text("split_ranges", "all");
            let pages = parse_ranges(expr, total).ok_or((
                "invalid_option",
                format!("页码范围「{expr}」无法解析，应形如 1-3,5,8-10 或 all"),
            ))?;
            // 把连续的页聚成一组，1-3 出一份、5 出一份、8-10 出一份
            let mut out: Vec<(String, Vec<u32>)> = Vec::new();
            let mut cur: Vec<u32> = Vec::new();
            for p in pages {
                if let Some(&last) = cur.last() {
                    if p != last + 1 {
                        out.push((label_of(&cur), std::mem::take(&mut cur)));
                    }
                }
                cur.push(p);
            }
            if !cur.is_empty() {
                out.push((label_of(&cur), cur));
            }
            out
        }
    };

    if chunks.is_empty() {
        return Err(("invalid_option", "拆分后没有任何输出".into()));
    }

    let stem = dst
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "part".into());
    let ext = dst
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "pdf".into());

    for (i, (label, pages)) in chunks.iter().enumerate() {
        let mut part = doc.clone();
        let all: Vec<u32> = part.get_pages().keys().copied().collect();
        let keep: Vec<u32> = all.iter().copied().filter(|p| pages.contains(p)).collect();
        let drop: Vec<u32> = all.iter().copied().filter(|p| !pages.contains(p)).collect();
        if !drop.is_empty() {
            part.delete_pages(&drop);
        }
        // 删完之后页码重排了，还要把保留的页按用户给定的顺序挂回去
        let after = part.get_pages();
        let order: Vec<u32> = pages
            .iter()
            .filter_map(|p| keep.iter().position(|k| k == p).map(|i| (i + 1) as u32))
            .collect();
        reorder_pages(&mut part, &order, &after)?;

        // 拆分产物写在目标旁边：报告.pdf → 报告-1-3.pdf。
        // 冲突在这里按**每个产物**分别解决——扫描阶段给的是命名模板，
        // 它并不知道会拆出几份、每份叫什么。
        let out_path = {
            let desired = if chunks.len() == 1 {
                dst.clone()
            } else {
                dst.with_file_name(format!("{stem}-{label}.{ext}"))
            };
            let dir = desired.parent().unwrap_or_else(|| Path::new("."));
            let stem_desired = desired
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "part".into());
            let mut claimed = std::collections::HashSet::new();
            let name = paths::pick_free_name(dir, &stem_desired, &ext, &mut claimed);
            dir.join(name)
        };
        save_to(part, &out_path.to_string_lossy())?;
        reporter.progress(spec.id, (i + 1) as f32 / chunks.len() as f32);
    }
    Ok(())
}

fn label_of(pages: &[u32]) -> String {
    match (pages.first(), pages.last()) {
        (Some(a), Some(b)) if a == b => format!("{a}"),
        (Some(a), Some(b)) => format!("{a}-{b}"),
        _ => "part".into(),
    }
}

/* ---------------------------------------------------------------- 取文本 */

fn extract_text(spec: &JobSpec, reporter: &Reporter) -> Result<(), (&'static str, String)> {
    let src = PathBuf::from(&spec.src);
    let dst = PathBuf::from(&spec.dst);

    let doc = load(&src)?;
    let total = doc.get_pages().len() as u32;
    if total == 0 {
        return Err(("corrupt", "PDF 里没有页面".into()));
    }

    let range = spec.text("page_range", "all");
    let pages = parse_ranges(range, total).ok_or((
        "invalid_option",
        format!("页码范围「{range}」无法解析，应形如 1-3,7 或 all"),
    ))?;

    reporter.progress(spec.id, 0.4);

    // lopdf 一次把所有页的文本取出来再切，这里按需要的页码范围过滤
    let text = doc
        .extract_text(&pages)
        .map_err(|e| ("corrupt", format!("文本提取失败：{e}")))?;

    reporter.progress(spec.id, 0.8);

    let tmp = temp_sibling(&dst);
    if let Err(e) = std::fs::write(paths::for_io(&tmp), text.as_bytes()) {
        let _ = std::fs::remove_file(paths::for_io(&tmp));
        return Err(("io", format!("写入失败：{e}")));
    }
    std::fs::rename(paths::for_io(&tmp), paths::for_io(&dst))
        .map_err(|e| ("io", format!("无法落地输出文件：{e}")))?;
    Ok(())
}

/* ---------------------------------------------------------------- 读写 */

/// 载入 PDF，把 lopdf 的错误翻译成用户能看懂的分类。
///
/// 加密 PDF 会得到 `Decryption` 或 `UnsupportedSecurityHandler`，
/// 这两种都要报成「需要密码」而不是笼统的「文件损坏」——对用户来说
/// 补一个密码和换一个文件是两件完全不同的事。
fn load(path: &Path) -> Result<Document, (&'static str, String)> {
    Document::load(paths::for_io(path)).map_err(|e| classify_lopdf(&e))
}

fn classify_lopdf(e: &lopdf::Error) -> (&'static str, String) {
    use lopdf::Error::*;
    match e {
        Decryption(_) | UnsupportedSecurityHandler(_) => {
            ("password", "PDF 已加密，需要密码才能处理".into())
        }
        Parse(inner) => ("corrupt", format!("PDF 结构损坏：{inner}")),
        InvalidOffset(_) | Xref(_) | ObjectNotFound(_) | ReferenceCycle(_) => {
            ("corrupt", format!("PDF 结构损坏：{e}"))
        }
        IO(io) if io.kind() == std::io::ErrorKind::NotFound => {
            ("not_found", "找不到源文件".into())
        }
        IO(io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
            ("access_denied", "没有权限读写该文件".into())
        }
        IO(io) => ("io", format!("读写失败：{io}")),
        other => ("unknown", format!("{other}")),
    }
}

fn save_to(mut doc: Document, dst: &str) -> Result<(), (&'static str, String)> {
    let dst = PathBuf::from(dst);
    let tmp = temp_sibling(&dst);

    // prune 掉已经没人引用的对象，否则删页/拆分后文件会带着上一版的包袱
    doc.prune_objects();
    doc.renumber_objects();

    // Document::save 的 Err 是 std::io::Error，不是 lopdf::Error
    if let Err(e) = doc.save(paths::for_io(&tmp)) {
        let _ = std::fs::remove_file(paths::for_io(&tmp));
        return Err(("io", format!("写入 PDF 失败：{e}")));
    }
    if let Err(e) = std::fs::rename(paths::for_io(&tmp), paths::for_io(&dst)) {
        let _ = std::fs::remove_file(paths::for_io(&tmp));
        return Err(("io", format!("无法落地输出文件：{e}")));
    }
    Ok(())
}

/* ---------------------------------------------------------------- 小工具 */

fn total_input_bytes(spec: &JobSpec) -> u64 {
    spec.src
        .split('|')
        .map(|p| file_len(Path::new(p)))
        .sum()
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(paths::for_io(p)).map(|m| m.len()).unwrap_or(0)
}

fn temp_sibling(dst: &Path) -> PathBuf {
    let ext = dst.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let name = dst.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    dst.with_file_name(format!("{name}.{ext}.fftmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_ranges("all", 5), Some(vec![1, 2, 3, 4, 5]));
        assert_eq!(parse_ranges("1-3,5", 5), Some(vec![1, 2, 3, 5]));
        assert_eq!(parse_ranges("2", 5), Some(vec![2]));
        // 越界页码被丢掉，而不是报错
        assert_eq!(parse_ranges("4-9", 5), Some(vec![4, 5]));
        // 去重并排序
        assert_eq!(parse_ranges("3,1,3", 5), Some(vec![1, 3]));
        // 非法输入
        assert_eq!(parse_ranges("abc", 5), None);
        assert_eq!(parse_ranges("3-1", 5), None);
        assert_eq!(parse_ranges("0", 5), None);
        // 全部越界等于没选
        assert_eq!(parse_ranges("9", 5), None);
    }

    #[test]
    fn parses_order() {
        assert_eq!(parse_order("3,1,2", 3), Some(vec![3, 1, 2]));
        // 越界要报错，不能默默吞掉——用户以为排好了，其实少了一页
        assert_eq!(parse_order("1,9", 3), None);
        assert_eq!(parse_order("x", 3), None);
    }

    #[test]
    fn parses_drops() {
        assert_eq!(parse_drops("1,3,5"), vec![1, 3, 5]);
        assert_eq!(parse_drops("2-4"), vec![2, 3, 4]);
        assert_eq!(parse_drops(""), Vec::<u32>::new());
        assert_eq!(parse_drops("2,2,1"), vec![1, 2]);
    }

    #[test]
    fn normalizes_rotation_to_right_angles() {
        assert_eq!(normalize_rotation(90), 90);
        assert_eq!(normalize_rotation(180), 180);
        assert_eq!(normalize_rotation(270), 270);
        assert_eq!(normalize_rotation(360), 0);
        assert_eq!(normalize_rotation(-90), 270);
        // 90 与 270 各自加 90 的结果
        assert_eq!(normalize_rotation(90 + 90), 180);
        assert_eq!(normalize_rotation(270 + 90), 0);
        // 非 90 倍数吸附到最近的
        assert_eq!(normalize_rotation(100), 90);
        assert_eq!(normalize_rotation(350), 0);
    }

    #[test]
    fn labels_chunks() {
        assert_eq!(label_of(&[1]), "1");
        assert_eq!(label_of(&[1, 2, 3]), "1-3");
        assert_eq!(label_of(&[]), "part");
    }
}
