//! 格式注册表：转换能力的唯一事实来源。
//!
//! 前端通过 `list_formats` 拉取这张表并据此动态渲染目标格式选单与参数面板，
//! 所以「哪些格式能转成哪些」只定义一遍，不会出现前后端漂移。
//!
//! 同格式转换（PNG → PNG）不在能力范围内，故 `targets` 一律不含自身。
//!
//! **这里只登记真正实现了的格式。** 可选却一定失败的条目比没有还糟——
//! 用户选了才在转换阶段撞到「暂不支持」。所以 AVIF 已从表中移除（`image`
//! crate 需要开 `avif` feature 才有编码器，那是 ravif 一整套依赖），
//! SVG 与 HEIC 也不在表内（栅格化要 resvg / 系统 HEIF 扩展，尚未接入）。

use serde::Serialize;

use crate::formats::Category;

/// 参数控件的类型。
///
/// **必须序列化成 PascalCase**：前端 `OptionPanel.jsx` 按 `'Slider'` /
/// `'Select'` 这样的字面量分支渲染控件。这里加 `rename_all = "lowercase"`
/// 会让每个参数都渲染不出控件——而且不报错，只是参数面板一片空白。
/// `tests/registry_shape.rs` 就是为守住这条约定而写的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum OptionKind {
    /// 滑杆，配 min/max/step
    Slider,
    /// 下拉选择，配 choices
    Select,
    /// 数字输入
    Number,
    /// 开关
    Toggle,
    /// 文本输入
    Text,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
}

/// 一个可调参数。`targets` 为空表示对所有目标格式都适用。
#[derive(Debug, Clone, Serialize)]
pub struct OptionDef {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: OptionKind,
    pub default: OptDefault,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<&'static [Choice]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<&'static str>,
    /// 仅在这些目标格式下出现；空 = 全部
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<&'static str>,
}

/// 默认值。用枚举而不是 Value，避免为一张静态表引入动态类型。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(untagged)]
pub enum OptDefault {
    Num(f64),
    Str(&'static str),
    Bool(bool),
}

#[derive(Debug, Clone, Serialize)]
pub struct FormatDef {
    pub id: &'static str,
    pub label: &'static str,
    pub category: Category,
    pub ext: &'static [&'static str],
    pub mime: &'static str,
    /// 可解码（能作为源）
    pub can_decode: bool,
    /// 可编码（能作为目标）
    pub can_encode: bool,
    /// 合法的目标格式；不含自身
    pub targets: Vec<&'static str>,
    pub options: Vec<OptionDef>,
}

/* ---------------------------------------------------------------- 参数预设 */

fn opt_quality(default: f64, targets: &[&'static str]) -> OptionDef {
    OptionDef {
        key: "quality",
        label: "质量",
        kind: OptionKind::Slider,
        default: OptDefault::Num(default),
        min: Some(1.0),
        max: Some(100.0),
        step: Some(1.0),
        unit: Some("%"),
        choices: None,
        hint: Some("数值越高越清晰、体积越大"),
        targets: targets.to_vec(),
    }
}

fn opt_resize() -> OptionDef {
    OptionDef {
        key: "resize_mode",
        label: "尺寸",
        kind: OptionKind::Select,
        default: OptDefault::Str("none"),
        min: None,
        max: None,
        step: None,
        unit: None,
        choices: Some(&[
            Choice { value: "none", label: "保持原尺寸" },
            Choice { value: "longest", label: "限制长边" },
            Choice { value: "width", label: "指定宽度" },
            Choice { value: "height", label: "指定高度" },
        ]),
        hint: None,
        targets: vec![],
    }
}

fn opt_resize_value() -> OptionDef {
    OptionDef {
        key: "resize_value",
        label: "目标像素",
        kind: OptionKind::Number,
        default: OptDefault::Num(1600.0),
        min: Some(16.0),
        max: Some(20000.0),
        step: Some(1.0),
        unit: Some("px"),
        choices: None,
        hint: None,
        targets: vec![],
    }
}

fn opt_strip_exif() -> OptionDef {
    OptionDef {
        key: "strip_exif",
        label: "移除 EXIF 元数据",
        kind: OptionKind::Toggle,
        default: OptDefault::Bool(true),
        min: None,
        max: None,
        step: None,
        unit: None,
        choices: None,
        hint: Some("含拍摄时间与 GPS 位置"),
        targets: vec![],
    }
}

/* ---------------------------------------------------------------- 注册表 */

const ALL_IMAGE_TARGETS: &[&str] = &["png", "jpeg", "webp", "gif", "bmp", "tiff", "ico"];

/// 每种图片格式各自能转的目标（排除自身）
fn image_targets(id: &str) -> Vec<&'static str> {
    ALL_IMAGE_TARGETS.iter().copied().filter(|t| *t != id).collect()
}

/// 质量参数只对**有损**编码器有意义。
///
/// WebP 不在列：`image` crate 的 WebP 编码器只做无损，`save_with_format`
/// 会把 quality 整个丢掉——实测 q10 与 q95 产出字节完全相同。与其放一个
/// 拖了没反应的滑杆，不如让它别出现在 WebP 的参数面板里。
///
/// 画质实测（128×128 噪声图，逐像素比对）：
/// `jpeg` q85 平均绝对差 9.4 / 最大 63；`webp` / `bmp` / `tiff` 无损，差 0；
/// `gif` 因为是 256 色，平均差 10.9。
fn image_options() -> Vec<OptionDef> {
    vec![
        opt_quality(85.0, &["jpeg"]),
        opt_resize(),
        opt_resize_value(),
        opt_strip_exif(),
    ]
}

pub fn registry() -> Vec<FormatDef> {
    let mut out = Vec::new();

    // ---------- 图片 ----------
    let images: [(&str, &str, &[&str], &str); 7] = [
        ("png", "PNG", &["png"], "image/png"),
        ("jpeg", "JPEG", &["jpg", "jpeg"], "image/jpeg"),
        ("webp", "WebP", &["webp"], "image/webp"),
        ("gif", "GIF", &["gif"], "image/gif"),
        ("bmp", "BMP", &["bmp"], "image/bmp"),
        ("tiff", "TIFF", &["tif", "tiff"], "image/tiff"),
        ("ico", "ICO", &["ico"], "image/x-icon"),
    ];

    for (id, label, ext, mime) in images {
        out.push(FormatDef {
            id,
            label,
            category: Category::Image,
            ext,
            mime,
            can_decode: true,
            can_encode: true,
            targets: image_targets(id),
            options: image_options(),
        });
    }

    // SVG 与 HEIC 不在这里：它们只能当源，而源侧的解码器还没接
    // （SVG 要 resvg，HEIC 要系统 HEIF 扩展）。登记了也只会让用户
    // 拖进来才发现转不了。接上解码器时再一起把 FormatDef 加回来。

    // ---------- 文档 ----------
    out.push(FormatDef {
        id: "docx",
        label: "Word 文档",
        category: Category::Document,
        ext: &["docx", "docm", "doc", "rtf"],
        mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        can_decode: true,
        can_encode: false,
        targets: vec!["pdf", "txt", "md", "html"],
        options: vec![
            engine_option(),
            OptionDef {
                key: "keep_images",
                label: "保留图片链接",
                kind: OptionKind::Toggle,
                default: OptDefault::Bool(true),
                min: None,
                max: None,
                step: None,
                unit: None,
                choices: None,
                hint: Some("仅在选择 Markdown / HTML 为目标时生效"),
                targets: vec!["md", "html"],
            },
        ],
    });

    out.push(FormatDef {
        id: "xlsx",
        label: "Excel 表格",
        category: Category::Document,
        ext: &["xlsx", "xlsm", "xls"],
        mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        can_decode: true,
        can_encode: false,
        targets: vec!["pdf", "csv", "tsv", "json"],
        options: vec![
            engine_option(),
            OptionDef {
                key: "sheet_mode",
                label: "工作表处理",
                kind: OptionKind::Select,
                default: OptDefault::Str("separate"),
                min: None,
                max: None,
                step: None,
                unit: None,
                choices: Some(&[
                    Choice { value: "separate", label: "每个工作表一个文件" },
                    Choice { value: "merge", label: "合并为一个文件" },
                    Choice { value: "first", label: "只取第一个工作表" },
                ]),
                hint: None,
                targets: vec!["csv", "tsv", "json"],
            },
            OptionDef {
                key: "header_row",
                label: "首行作为表头",
                kind: OptionKind::Toggle,
                default: OptDefault::Bool(true),
                min: None,
                max: None,
                step: None,
                unit: None,
                choices: None,
                hint: Some("转 JSON 时生效"),
                targets: vec!["json"],
            },
        ],
    });

    out.push(FormatDef {
        id: "pptx",
        label: "PowerPoint 演示",
        category: Category::Document,
        ext: &["pptx", "pptm", "ppt"],
        mime: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        can_decode: true,
        can_encode: false,
        // 只有 pdf：幻灯片转图要走 `Presentation.Export`，与现在的
        // `SaveAs($dst, ppSaveAsPDF)` 是两条不同的 COM 路径，还没写。
        // 那对滑杆（96/150/300 DPI）跟着一起撤了——它只为那条路径存在。
        targets: vec!["pdf"],
        options: vec![engine_option()],
    });

    // ---------- PDF ----------
    out.push(FormatDef {
        id: "pdf",
        label: "PDF 文档",
        category: Category::Document,
        ext: &["pdf"],
        mime: "application/pdf",
        can_decode: true,
        can_encode: true,
        // `@` 前缀的是**动作**而不是格式：它们产出的还是 PDF（或别的），
        // 但需要用户额外给参数。放在同一个选单里，前端按前缀分支渲染。
        //
        // 这里没有 png / jpeg：PDF→图片要先把页面渲染成位图，需要 pdfium
        // （C++ 库，要分发一个 7MB 的 DLL 并处理动态绑定）。与其让用户选了
        // 才撞到「尚未接入」，不如先不列出来。
        targets: vec!["txt", "@organize", "@split", "@merge"],
        options: vec![
            OptionDef {
                key: "pdf_mode",
                label: "处理方式",
                kind: OptionKind::Select,
                default: OptDefault::Str("keep"),
                min: None,
                max: None,
                step: None,
                unit: None,
                choices: Some(&[
                    Choice { value: "keep", label: "整理页面（保留全部）" },
                    Choice { value: "split", label: "拆分" },
                ]),
                hint: Some("选「拆分」时用下面的拆分方式；选「整理」时用删页/重排/旋转"),
                targets: vec!["@organize", "@split"],
            },
            OptionDef {
                key: "pdf_drop",
                label: "删除页面",
                kind: OptionKind::Text,
                default: OptDefault::Str(""),
                min: None,
                max: None,
                step: None,
                unit: None,
                choices: None,
                hint: Some("留空表示不删；可写 3 或 3-5，逗号分隔"),
                targets: vec!["@organize"],
            },
            OptionDef {
                key: "pdf_order",
                label: "页面顺序",
                kind: OptionKind::Text,
                default: OptDefault::Str(""),
                min: None,
                max: None,
                step: None,
                unit: None,
                choices: None,
                hint: Some("留空表示保持原顺序；可写 3,1,2 把第 3 页提到最前"),
                targets: vec!["@organize"],
            },
            OptionDef {
                key: "pdf_rotate",
                label: "整份旋转",
                kind: OptionKind::Select,
                default: OptDefault::Str("0"),
                min: None,
                max: None,
                step: None,
                unit: None,
                choices: Some(&[
                    Choice { value: "0", label: "不旋转" },
                    Choice { value: "90", label: "顺时针 90°" },
                    Choice { value: "180", label: "180°" },
                    Choice { value: "270", label: "逆时针 90°" },
                ]),
                hint: None,
                targets: vec!["@organize"],
            },
            OptionDef {
                key: "split_mode",
                label: "拆分方式",
                kind: OptionKind::Select,
                default: OptDefault::Str("ranges"),
                min: None,
                max: None,
                step: None,
                unit: None,
                choices: Some(&[
                    Choice { value: "ranges", label: "按页码范围分组" },
                    Choice { value: "every", label: "每 N 页一份" },
                ]),
                hint: None,
                targets: vec!["@split"],
            },
            OptionDef {
                key: "split_ranges",
                label: "页码范围",
                kind: OptionKind::Text,
                default: OptDefault::Str("all"),
                min: None,
                max: None,
                step: None,
                unit: None,
                choices: None,
                hint: Some("all 表示全部；也可写 1-3,5,8-10，连续区间会各自成一份"),
                targets: vec!["@split"],
            },
            OptionDef {
                key: "split_every",
                label: "每份页数",
                kind: OptionKind::Number,
                default: OptDefault::Num(1.0),
                min: Some(1.0),
                max: Some(5000.0),
                step: Some(1.0),
                unit: Some("页"),
                choices: None,
                hint: None,
                targets: vec!["@split"],
            },
            OptionDef {
                key: "page_range",
                label: "页码范围",
                kind: OptionKind::Text,
                default: OptDefault::Str("all"),
                min: None,
                max: None,
                step: None,
                unit: None,
                choices: None,
                hint: Some("all 表示全部；也可写 1-3,7"),
                targets: vec!["txt"],
            },
        ],
    });

    // ---------- 数据 ----------
    out.push(FormatDef {
        id: "csv",
        label: "CSV 表格",
        category: Category::Data,
        ext: &["csv"],
        mime: "text/csv",
        can_decode: true,
        can_encode: true,
        targets: vec!["json", "tsv", "xlsx"],
        options: vec![
            OptionDef {
                key: "delimiter",
                label: "分隔符",
                kind: OptionKind::Select,
                default: OptDefault::Str("auto"),
                min: None,
                max: None,
                step: None,
                unit: None,
                choices: Some(&[
                    Choice { value: "auto", label: "自动识别" },
                    Choice { value: ",", label: "逗号 ," },
                    Choice { value: ";", label: "分号 ;" },
                    Choice { value: "\t", label: "制表符" },
                ]),
                hint: None,
                targets: vec![],
            },
            encoding_option(),
        ],
    });

    out.push(FormatDef {
        id: "json",
        label: "JSON 数据",
        category: Category::Data,
        ext: &["json"],
        mime: "application/json",
        can_decode: true,
        can_encode: true,
        targets: vec!["csv", "tsv", "xlsx"],
        options: vec![OptionDef {
            key: "json_shape",
            label: "顶层结构",
            kind: OptionKind::Select,
            default: OptDefault::Str("array"),
            min: None,
            max: None,
            step: None,
            unit: None,
            choices: Some(&[
                Choice { value: "array", label: "对象数组" },
                Choice { value: "auto", label: "自动识别" },
            ]),
            hint: None,
            targets: vec![],
        }],
    });

    out
}

fn engine_option() -> OptionDef {
    OptionDef {
        key: "pdf_engine",
        label: "转换引擎",
        kind: OptionKind::Select,
        default: OptDefault::Str("auto"),
        min: None,
        max: None,
        step: None,
        unit: None,
        choices: Some(&[
            Choice { value: "auto", label: "自动选择（优先保真）" },
            Choice { value: "office", label: "Microsoft Office" },
            Choice { value: "wps", label: "WPS Office" },
            Choice { value: "libreoffice", label: "LibreOffice" },
        ]),
        hint: Some("仅在选择 PDF 为目标时生效"),
        targets: vec!["pdf"],
    }
}

fn encoding_option() -> OptionDef {
    OptionDef {
        key: "encoding",
        label: "输出编码",
        kind: OptionKind::Select,
        default: OptDefault::Str("utf-8"),
        min: None,
        max: None,
        step: None,
        unit: None,
        choices: Some(&[
            Choice { value: "utf-8", label: "UTF-8" },
            Choice { value: "utf-8-bom", label: "UTF-8 with BOM" },
            Choice { value: "gbk", label: "GBK" },
        ]),
        hint: Some("UTF-8 with BOM 在旧版 Excel 里打开不乱码"),
        targets: vec![],
    }
}

/// 查一个格式定义
pub fn find(id: &str) -> Option<FormatDef> {
    registry().into_iter().find(|f| f.id == id)
}

/// 按扩展名反查格式 id。
///
/// 嗅探认不出内容时的兜底——比起把文件静默丢掉，按扩展名归类后让转换
/// 给出真实报错（「文档已损坏」）对用户有用得多。扩展名与内容不符的情况
/// 在 `formats::sniff` 那一层已经处理，这里只在内容完全无法识别时生效。
pub fn id_for_ext(ext: &str) -> Option<&'static str> {
    let e = ext.to_ascii_lowercase();
    registry()
        .into_iter()
        .find(|f| f.ext.iter().any(|x| *x == e))
        .map(|f| f.id)
}

/// 这个 id 是不是一个合法**目标格式**。
///
/// 注意不能只查 `find()`：`txt` / `md` / `html` / `tsv` 是别人能转到的
/// 目标，但本身没有（也不需要有）独立的 `FormatDef`——转换器按目标分派，
/// 碰到 `txt` 就走文本提取，不需要 `txt` 自己的定义。
pub fn is_known_target(id: &str) -> bool {
    let reg = registry();
    reg.iter().any(|f| f.id == id)
        || reg.iter().any(|f| f.targets.iter().any(|t| *t == id))
}

/// 当前选中的若干源格式能共同转换到的目标。
/// 交集为空时返回空表，前端据此显示「没有共同目标」并提示逐文件指定。
pub fn common_targets(src_ids: &[String]) -> Vec<&'static str> {
    let reg = registry();
    let mut result: Option<Vec<&'static str>> = None;

    for id in src_ids {
        let Some(def) = reg.iter().find(|f| f.id == id) else {
            return vec![];
        };
        let set = def.targets.clone();
        result = Some(match result {
            None => set,
            Some(acc) => acc.into_iter().filter(|t| set.contains(t)).collect(),
        });
    }

    result.unwrap_or_default()
}
