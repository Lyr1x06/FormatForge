//! 格式识别。
//!
//! 用魔数嗅探真实格式，而不是信扩展名——用户把 .png 改名成 .jpg 是常事，
//! 按扩展名去解码只会得到一句莫名其妙的报错。
//!
//! Office 的 OOXML 家族（docx/xlsx/pptx）本质是 zip，`infer` 一律报 zip，
//! 所以要额外看一眼压缩包里的顶层目录名才能分辨。

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Image,
    Document,
    Data,
    Unknown,
}

impl Category {
    /// 判定某个格式 id 属于哪个大类
    pub fn of(id: &str) -> Category {
        const IMAGES: [&str; 13] = [
            "png", "jpeg", "webp", "avif", "gif", "bmp", "tiff", "svg", "ico", "heic", "heif",
            "jxl", "tga",
        ];
        const DATA: [&str; 5] = ["csv", "tsv", "json", "xlsx_sheet", "ndjson"];

        if IMAGES.contains(&id) {
            Category::Image
        } else if DATA.contains(&id) {
            Category::Data
        } else if id.is_empty() {
            Category::Unknown
        } else {
            Category::Document
        }
    }
}

/// 嗅探结果
#[derive(Debug, Clone, Serialize)]
pub struct Sniff {
    /// 规范化的格式 id（如 "png" / "docx" / "csv"）
    pub id: Option<String>,
    pub label: &'static str,
    pub category: Category,
    /// 扩展名与内容不符时为 true
    pub mismatch: bool,
}

/// OOXML 三兄弟的顶层目录名
const OOXML_MARKERS: [(&str, &str, &str); 3] = [
    (r"word/", "docx", "Word 文档"),
    (r"xl/", "xlsx", "Excel 表格"),
    (r"ppt/", "pptx", "PowerPoint 演示"),
];

/// 兜底：`infer` 认不出来时的纯扩展名映射。
/// 主要覆盖 infer 不做的文本类格式。
const BY_EXT: [(&str, &str, Category, &str); 10] = [
    ("csv", "csv", Category::Data, "CSV 表格"),
    ("tsv", "tsv", Category::Data, "TSV 表格"),
    ("json", "json", Category::Data, "JSON 数据"),
    ("txt", "txt", Category::Document, "纯文本"),
    ("md", "md", Category::Document, "Markdown"),
    ("markdown", "md", Category::Document, "Markdown"),
    ("html", "html", Category::Document, "HTML"),
    ("htm", "html", Category::Document, "HTML"),
    ("doc", "doc", Category::Document, "Word 97-2003"),
    ("xls", "xls", Category::Document, "Excel 97-2003"),
];

/// 判定某个格式 id 属于哪个大类
pub fn category_of(id: &str) -> Category {
    Category::of(id)
}

pub fn label_of(id: &str) -> &'static str {
    match id {
        "png" => "PNG 图片",
        "jpeg" => "JPEG 图片",
        "webp" => "WebP 图片",
        "avif" => "AVIF 图片",
        "gif" => "GIF 图片",
        "bmp" => "BMP 图片",
        "tiff" => "TIFF 图片",
        "svg" => "SVG 矢量图",
        "ico" => "ICO 图标",
        "heic" | "heif" => "HEIC 图片",
        "pdf" => "PDF 文档",
        "docx" => "Word 文档",
        "xlsx" => "Excel 表格",
        "pptx" => "PowerPoint 演示",
        "csv" => "CSV 表格",
        "tsv" => "TSV 表格",
        "json" => "JSON 数据",
        "txt" => "纯文本",
        "md" => "Markdown",
        "html" => "HTML",
        "doc" => "Word 97-2003",
        "xls" => "Excel 97-2003",
        "ppt" => "PowerPoint 97-2003",
        _ => "未知格式",
    }
}

/// `infer` 报出来的 zip 需要进一步分辨是哪一种 OOXML。
fn sniff_ooxml(path: &std::path::Path) -> Option<&'static str> {
    use std::io::Read;

    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 8192];
    let n = f.read(&mut buf).ok()?;
    let head = &buf[..n];

    // zip 的本地文件头里文件名是明文存储的，扫一下就能认出家族
    for (marker, id, _) in OOXML_MARKERS {
        if head
            .windows(marker.len())
            .any(|w| w == marker.as_bytes())
        {
            return Some(id);
        }
    }
    None
}

pub fn sniff(path: &std::path::Path, ext: &str) -> Sniff {
    let ext_lower = ext.to_ascii_lowercase();

    // 先用 infer 看魔数
    let guess = infer::get_from_path(path).ok().flatten();
    let mut id: Option<String> = None;
    let mut sniffed = false;

    if let Some(t) = guess {
        let ief = t.extension().to_ascii_lowercase();
        if ief == "zip" {
            // 可能是 OOXML，也可能真就是个压缩包
            if let Some(real) = sniff_ooxml(path) {
                id = Some(real.to_string());
                sniffed = true;
            } else if matches!(ext_lower.as_str(), "docx" | "xlsx" | "pptx" | "docm" | "xlsm" | "pptm")
            {
                // 扩展名声称是 Office 文档，压缩包里却没有对应目录。
                // 这是个 zip，但不是有效的 Office 文档——按扩展名的类别归类，
                // 让转换阶段给出「文档已损坏」而不是在这里静默丢弃。
                id = Some(ext_lower.clone());
            } else {
                id = Some("zip".into());
                sniffed = true;
            }
        } else {
            // infer 对 jpeg 报 "jpg"，统一到 "jpeg"
            id = Some(if ief == "jpg" { "jpeg".into() } else { ief });
            sniffed = true;
        }
    }

    // infer 认不出来（多为纯文本格式）→ 退回扩展名
    if id.is_none() {
        for (e, fid, _, _) in BY_EXT {
            if e == ext_lower {
                id = Some(fid.to_string());
                sniffed = true;
                break;
            }
        }
    }

    let Some(id) = id else {
        return Sniff {
            id: None,
            label: label_of(&ext_lower),
            category: Category::Unknown,
            mismatch: false,
        };
    };

    // 只有真的看过内容（而不是纯粹按扩展名归类）才谈得上「相符/不符」
    let mismatch = sniffed && !ext_lower.is_empty() && !ext_matches(&ext_lower, &id);

    Sniff {
        label: label_of(&id),
        category: category_of(&id),
        id: Some(id),
        mismatch,
    }
}

/// 扩展名与嗅探出的 id 是否兼容
fn ext_matches(ext: &str, id: &str) -> bool {
    if ext == id {
        return true;
    }
    match id {
        // 有多个常见扩展名的格式
        "jpeg" => matches!(ext, "jpg" | "jpeg" | "jpe" | "jfif"),
        "tiff" => matches!(ext, "tif" | "tiff"),
        "heic" => matches!(ext, "heic" | "heif"),
        "md" => matches!(ext, "md" | "markdown"),
        "html" => matches!(ext, "html" | "htm"),
        // Office 2007+ 的宏/模板变体
        "docx" => matches!(ext, "docx" | "docm" | "dotx" | "dotm"),
        "xlsx" => matches!(ext, "xlsx" | "xlsm" | "xltx" | "xltm"),
        "pptx" => matches!(ext, "pptx" | "pptm" | "potx" | "potm"),
        _ => false,
    }
}

