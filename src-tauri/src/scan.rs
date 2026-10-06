//! 输入扫描：把用户拖进来的路径展开成可转换的作业规格。
//!
//! 这一步做三件容易出错的事：
//!   1. 文件夹递归——`follow_links` 必须关，Windows 的目录联接会造成无限递归。
//!   2. 魔数嗅探——按内容而不是扩展名决定合法目标格式。
//!   3. 输出命名——批内预占名字，避免 `photo.png` 与 `photo.jpg` 同时转 WebP 撞名。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::formats::{self, Category};
use crate::job::JobSpec;
use crate::paths;

/// 递归时要跳过的目录名
const SKIP_DIRS: &[&str] = &[
    "$RECYCLE.BIN",
    "System Volume Information",
    "node_modules",
    ".git",
    ".svn",
    ".hg",
    "Windows",
    "AppData",
];

/// 单次扫描的文件数上限。再多就不是「批量」而是「整盘」了。
const MAX_FILES: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScanCategory {
    Image,
    Document,
    Data,
    Unknown,
}

impl From<Category> for ScanCategory {
    fn from(c: Category) -> Self {
        match c {
            Category::Image => ScanCategory::Image,
            Category::Document => ScanCategory::Document,
            Category::Data => ScanCategory::Data,
            Category::Unknown => ScanCategory::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannedFile {
    /// 稳定的行 id，前端据此索引状态
    pub id: u32,
    pub path: String,
    /// 展示用文件名
    pub name: String,
    /// 相对投放文件夹的路径，用于区分同名文件
    pub rel: Option<String>,
    pub bytes: u64,
    /// 嗅探到的真实格式
    pub format: String,
    pub format_label: String,
    pub category: ScanCategory,
    /// 扩展名与内容不符
    pub mismatch: bool,
    /// 该格式能转到的目标（已排除自身）
    pub targets: Vec<String>,
    /// 无法识别或格式不支持转换
    pub unsupported: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub files: Vec<ScannedFile>,
    /// 跳过的数量与原因，要如实告诉用户
    pub skipped: Vec<SkippedGroup>,
    /// 当前这批的共同目标格式；为空表示没有交集
    pub common_targets: Vec<String>,
    pub total_bytes: u64,
    pub truncated: bool,
    /// 建议的输出目录（第一个源文件所在目录）。用户还没指定时用它打底。
    pub suggested_output_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedGroup {
    pub reason: String,
    pub count: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScanRequest {
    pub paths: Vec<String>,
    /// 是否递归子文件夹
    #[serde(default = "default_recurse")]
    pub recurse: bool,
    #[serde(default = "default_depth")]
    pub max_depth: usize,
}

fn default_recurse() -> bool {
    true
}
fn default_depth() -> usize {
    8
}

/// 扫描入口。不转换，只收集与识别。
pub fn scan(req: &ScanRequest) -> ScanResult {
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let mut truncated = false;

    let mut hidden_count = 0usize;
    let mut unsupported_ext = 0usize;
    let mut unreadable = 0usize;

    let mut next_id: u32 = 0;
    let mut seen: HashSet<String> = HashSet::new();

    for raw in &req.paths {
        let p = PathBuf::from(raw);
        if p.is_dir() {
            let root_name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| raw.clone());

            let walker = walkdir::WalkDir::new(&p)
                .max_depth(if req.recurse { req.max_depth } else { 1 })
                // 必须 false：Windows 的目录联接会让 walkdir 无限递归
                .follow_links(false)
                .into_iter();

            for entry in walker.filter_entry(|e| {
                if e.file_type().is_dir() {
                    let name = e.file_name().to_string_lossy();
                    // 顶层投放目录本身不能被自己跳过
                    if e.depth() == 0 {
                        return true;
                    }
                    if SKIP_DIRS.contains(&name.as_ref()) {
                        return false;
                    }
                    if name.starts_with('.') {
                        return false;
                    }
                }
                true
            }) {
                let Ok(entry) = entry else {
                    unreadable += 1;
                    continue;
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                if files.len() >= MAX_FILES {
                    truncated = true;
                    break;
                }
                let path = entry.path();
                if is_hidden(path) {
                    hidden_count += 1;
                    continue;
                }
                let key = path.to_string_lossy().to_ascii_lowercase();
                if !seen.insert(key) {
                    continue;
                }

                let rel = path
                    .strip_prefix(&p)
                    .ok()
                    .and_then(|r| r.parent())
                    .filter(|d| !d.as_os_str().is_empty())
                    .map(|d| format!("{root_name}\\{}", d.to_string_lossy()));

                match build_file(next_id, path, rel.as_deref()) {
                    Built::Ok(f) => {
                        next_id += 1;
                        files.push(f);
                    }
                    Built::Unsupported => unsupported_ext += 1,
                    Built::Unreadable => unreadable += 1,
                }
            }
        } else if p.is_file() {
            if files.len() >= MAX_FILES {
                truncated = true;
                break;
            }
            let key = p.to_string_lossy().to_ascii_lowercase();
            if !seen.insert(key) {
                continue;
            }
            match build_file(next_id, &p, None) {
                Built::Ok(f) => {
                    next_id += 1;
                    files.push(f);
                }
                Built::Unsupported => unsupported_ext += 1,
                Built::Unreadable => unreadable += 1,
            }
        } else {
            unreadable += 1;
        }
    }

    if hidden_count > 0 {
        skipped.push(SkippedGroup { reason: "隐藏文件".into(), count: hidden_count });
    }
    if unsupported_ext > 0 {
        skipped.push(SkippedGroup { reason: "不支持的格式".into(), count: unsupported_ext });
    }
    if unreadable > 0 {
        skipped.push(SkippedGroup { reason: "无法读取".into(), count: unreadable });
    }
    if truncated {
        skipped.push(SkippedGroup {
            reason: format!("超出 {MAX_FILES} 个文件的上限"),
            count: 1,
        });
    }

    // 共同目标：只在可转换的文件之间求交集
    let ids: Vec<String> = files
        .iter()
        .filter(|f| !f.unsupported)
        .map(|f| f.format.clone())
        .collect();
    let common = crate::registry::common_targets(&ids);

    // 建议输出目录：所有源文件的共同父目录；只有一个文件时就是它所在的目录
    let suggested_output_dir = suggest_output_dir(&files);

    ScanResult {
        total_bytes: files.iter().map(|f| f.bytes).sum(),
        common_targets: common.into_iter().map(String::from).collect(),
        files,
        skipped,
        truncated,
        suggested_output_dir,
    }
}

/// 取所有源文件的共同父目录。全在同一个文件夹时就是那个文件夹。
fn suggest_output_dir(files: &[ScannedFile]) -> Option<String> {
    let mut dirs = files.iter().filter_map(|f| {
        PathBuf::from(&f.path).parent().map(|p| p.to_path_buf())
    });

    let mut common = dirs.next()?;
    for d in dirs {
        while !d.starts_with(&common) {
            match common.parent() {
                Some(p) => common = p.to_path_buf(),
                None => return Some(paths::for_display(&common)),
            }
        }
    }
    Some(paths::for_display(&common))
}

enum Built {
    Ok(ScannedFile),
    Unsupported,
    Unreadable,
}

fn build_file(id: u32, path: &Path, rel: Option<&str>) -> Built {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return Built::Unreadable;
    }

    let bytes = match std::fs::metadata(paths::for_io(path)) {
        Ok(m) => m.len(),
        Err(_) => return Built::Unreadable,
    };

    let (_, ext) = paths::split_ext(path);
    let sniff = formats::sniff(path, &ext);

    // 内容认不出来时按扩展名归类。直接当成「不支持」把这个文件静默排除，
    // 用户看到的只是「拖进来 20 个，队列里只有 19 个」，没有任何解释；
    // 归类后让它走转换，会得到一句「文档已损坏」的明确报错。
    let fmt_id = match sniff.id.clone() {
        Some(id) => id,
        None => match crate::registry::id_for_ext(&ext) {
            Some(id) => id.to_string(),
            None => return Built::Unsupported,
        },
    };

    let Some(def) = crate::registry::find(&fmt_id) else {
        return Built::Unsupported;
    };
    if !def.can_decode {
        return Built::Unsupported;
    }

    Built::Ok(ScannedFile {
        id,
        path: paths::for_display(path),
        name,
        rel: rel.map(String::from),
        bytes,
        format: fmt_id,
        format_label: def.label.to_string(),
        category: sniff.category.into(),
        mismatch: sniff.mismatch,
        targets: def.targets.iter().map(|s| s.to_string()).collect(),
        unsupported: false,
    })
}

/// Windows 隐藏/系统属性
fn is_hidden(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    name.starts_with('.')
}

/// 用户选的目标可能是**格式**（`csv`）也可能是**动作**（`@split`）。
/// 动作要翻译成「产出的具体格式 + 注入的参数」，作业本身只认格式。
enum Target {
    /// 目标本身就是格式，直接当作业的输出格式
    Format(String),
    /// 动作：产出格式固定，另需注入参数
    Action {
        dst_format: &'static str,
        inject: Vec<(&'static str, &'static str)>,
    },
}

/// 认不出来返回 None。格式类的目标用注册表校验，避免前端传了个
/// 不存在的 id 就静默产出一堆空作业。
fn resolve_target(target: &str) -> Option<Target> {
    let action = |dst_format, inject| Some(Target::Action { dst_format, inject });

    match target {
        "@organize" => action("pdf", vec![("pdf_op", "organize")]),
        "@split" => action("pdf", vec![("pdf_op", "split")]),
        "@merge" => action("pdf", vec![("pdf_op", "merge")]),
        // 注册表是目标格式的唯一来源，不认识就拒绝。
        // 注意校验的是「能不能当目标」而不是「有没有自己的 FormatDef」——
        // txt/md/html/tsv 是别人能转到的目标，但不单独定义。
        _ if crate::registry::is_known_target(target) => Some(Target::Format(target.to_string())),
        _ => None,
    }
}

/// 按用户选定的目标，把扫描结果规划成可执行的作业。
///
/// 命名在这里一次性定完：批内同名（`photo.png` 与 `photo.jpg` 都转 WebP）
/// 也在这层解决，否则两个作业会抢同一个输出路径。
pub fn plan(
    files: &[ScannedFile],
    target: &str,
    output_dir: &Path,
    options: &HashMap<String, serde_json::Value>,
    src_stem_of: impl Fn(&ScannedFile) -> String,
) -> (Vec<JobSpec>, Vec<String>) {
    let mut notices = Vec::new();

    let (dst_format, inject): (String, Vec<(&'static str, &'static str)>) =
        match resolve_target(target) {
            Some(Target::Format(id)) => (id, Vec::new()),
            Some(Target::Action { dst_format, inject }) => (dst_format.to_string(), inject),
            None => {
                notices.push(format!("无法识别的目标：{target}"));
                return (Vec::new(), notices);
            }
        };

    // 合并是 N→1，不进单文件管线
    if target == "@merge" {
        return plan_merge(files, &dst_format, output_dir, options, notices);
    }

    // 动作注入的参数与用户给的合并，用户显式设置的优先
    let mut effective = options.clone();
    for (k, v) in &inject {
        effective
            .entry((*k).to_string())
            .or_insert_with(|| serde_json::Value::String((*v).to_string()));
    }

    let mut specs = Vec::new();
    let mut claimed: HashSet<String> = HashSet::new();
    let out_dir_str = output_dir.to_string_lossy().to_string();

    for f in files {
        if f.unsupported {
            continue;
        }
        if !f.targets.iter().any(|t| t == target) {
            notices.push(format!(
                "{}：{} 无法转为 {}",
                f.name, f.format_label, target
            ));
            continue;
        }

        let stem = src_stem_of(f);

        // 拆分是 1→N，产物名由转换器按页码范围推导（`报告-1-3.pdf`），
        // 这里给的 dst 只是**命名模板**。若在这里就把 `报告.pdf` 判为冲突
        // 而改成 `报告 (2).pdf`，拆出来的就变成 `报告 (2)-1-3.pdf`——
        // 用户看到会莫名其妙。所以拆分的冲突留给转换器按每个产物分别解决。
        let candidate = if target == "@split" {
            format!("{stem}.{dst_format}")
        } else {
            paths::pick_free_name(output_dir, &stem, &dst_format, &mut claimed)
        };
        claimed.insert(candidate.to_ascii_lowercase());

        let dst = output_dir.join(&candidate);
        let src_path = PathBuf::from(&f.path);

        if paths::over_budget(&src_path) > 0 || paths::over_budget(&dst) > 0 {
            notices.push(format!("{}：路径过长，已启用长路径兼容模式", f.name));
        }

        specs.push(JobSpec {
            id: f.id,
            src: f.path.clone(),
            src_format: f.format.clone(),
            dst_format: dst_format.clone(),
            dst: paths::for_display(&dst),
            rel: f.rel.clone(),
            options: effective.clone(),
        });
    }

    if !out_dir_str.is_empty() {
        notices.push(format!("输出目录：{out_dir_str}"));
    }
    (specs, notices)
}

/// 合并：把这一批 PDF 收成**一个**作业，源路径用 `|` 串起来。
///
/// 这不是单文件转换，所以不走上面那条循环——故意的，因为 N→1 的进度、
/// 命名、失败语义都跟 1→1 不一样。
fn plan_merge(
    files: &[ScannedFile],
    dst_format: &str,
    output_dir: &Path,
    options: &HashMap<String, serde_json::Value>,
    mut notices: Vec<String>,
) -> (Vec<JobSpec>, Vec<String>) {
    let pdfs: Vec<&ScannedFile> = files
        .iter()
        .filter(|f| !f.unsupported && f.format == "pdf")
        .collect();

    if pdfs.len() < 2 {
        notices.push("合并需要至少两个 PDF 文件".into());
        return (Vec::new(), notices);
    }

    // 输出名取第一个文件的主名
    let stem = Path::new(&pdfs[0].name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "merged".into());
    let candidate = paths::pick_free_name(
        output_dir,
        &stem,
        "pdf",
        &mut HashSet::new(),
    );
    let dst = output_dir.join(candidate);

    notices.push(format!(
        "将把 {} 个 PDF 按添加顺序合并为一个文件",
        pdfs.len()
    ));
    notices.push(format!("输出目录：{}", output_dir.to_string_lossy()));

    let src = pdfs
        .iter()
        .map(|f| f.path.clone())
        .collect::<Vec<_>>()
        .join("|");

    let mut merged_options = options.clone();
    merged_options.insert("pdf_op".into(), serde_json::Value::String("merge".into()));

    (
        vec![JobSpec {
            // 用第一个文件的 id 当作业 id，前端能把事件对回某一行
            id: pdfs[0].id,
            src,
            src_format: crate::pdf::MERGE_SRC.to_string(),
            dst_format: dst_format.to_string(),
            dst: paths::for_display(&dst),
            rel: None,
            options: merged_options,
        }],
        notices,
    )
}
