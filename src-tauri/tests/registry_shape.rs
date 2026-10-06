//! 注册表序列化出来的 JSON，形状必须是前端读得懂的那一份。
//!
//! 前端 `OptionPanel.jsx` 按 `kind` 的 PascalCase 值分支（`Slider` / `Select` …），
//! `TargetPicker` 按 `targets` 数组分组。这些约定跨了语言边界，编译器管不到——
//! serde 的 `rename_all` 一旦调整，前端会静默退化（比如所有参数都不显示）。
//! 这个测试就是那道防线。

use format_forge_lib::testkit::list_formats_json;

#[test]
fn formats_serialize_with_the_shape_the_frontend_reads() {
    let json = list_formats_json();
    let arr = json.as_array().expect("list_formats 必须是数组");
    assert!(!arr.is_empty());

    for f in arr {
        let obj = f.as_object().expect("每个格式必须是对象");
        for key in ["id", "label", "category", "ext", "mime", "can_decode", "can_encode", "targets", "options"] {
            assert!(obj.contains_key(key), "格式缺少字段 {key}：{f}");
        }

        assert!(obj["category"].is_string());
        assert!(obj["ext"].is_array());
        assert!(obj["targets"].is_array());

        for o in obj["options"].as_array().unwrap() {
            let oo = o.as_object().expect("参数必须是对象");
            for key in ["key", "label", "kind", "default"] {
                assert!(oo.contains_key(key), "参数缺少字段 {key}：{o}");
            }

            // 前端按这四个 PascalCase 值分支渲染控件
            let kind = oo["kind"].as_str().unwrap();
            assert!(
                ["Slider", "Select", "Number", "Toggle", "Text"].contains(&kind),
                "前端不认识的参数类型 {kind}：{o}"
            );

            // Select 必须给 choices，否则下拉是空的
            if kind == "Select" {
                let choices = oo["choices"].as_array().unwrap_or_else(|| {
                    panic!("Select 参数缺 choices：{o}");
                });
                assert!(!choices.is_empty(), "Select 的 choices 不能为空：{o}");
                for c in choices {
                    assert!(c.get("value").is_some() && c.get("label").is_some(), "choice 缺字段：{c}");
                }
            }

            // Slider 必须有 min/max，否则滑杆的取值域是 undefined
            if kind == "Slider" {
                assert!(oo["min"].is_number() && oo["max"].is_number(), "Slider 缺 min/max：{o}");
            }
        }
    }
}

#[test]
fn targets_never_include_self() {
    let json = list_formats_json();
    for f in json.as_array().unwrap() {
        let id = f["id"].as_str().unwrap();
        for t in f["targets"].as_array().unwrap() {
            assert_ne!(t.as_str().unwrap(), id, "{id} 的 targets 里不该含自己");
        }
    }
}

/// 每个非 `@` 目标都必须能落地到某个地方。
///
/// 「能落地」有两种：要么注册表里真有这个格式（`xlsx`、`webp`），
/// 要么它是某个格式的目标（`txt`、`html`、`tsv` —— 这些不需要自己
/// 有 FormatDef，转换器按目标分派就够了）。
///
/// 这条断言的价值：往注册表里加目标很容易，但忘了在后端实现分支的话，
/// 用户会在运行时才撞到「暂不支持」。
#[test]
fn every_target_is_implemented() {
    let reg = list_formats_json();
    let arr = reg.as_array().unwrap();

    // 所有出现过的 id：格式自己的 id + 所有 targets 里出现过的值
    let mut known: Vec<String> = Vec::new();
    for f in arr {
        known.push(f["id"].as_str().unwrap().to_string());
        for t in f["targets"].as_array().unwrap() {
            let t = t.as_str().unwrap();
            if !t.starts_with('@') {
                known.push(t.to_string());
            }
        }
    }

    for f in arr {
        for t in f["targets"].as_array().unwrap() {
            let t = t.as_str().unwrap();
            if t.starts_with('@') {
                continue;
            }
            assert!(
                known.iter().any(|k| k == t),
                "目标 {t} 既不是格式，也没被任何格式声明过"
            );
        }
    }
}

/// 注册表里声明的每个参数，后端都真的读它。
///
/// 这是上一条断言的**参数版**：能力表里的 `targets` 有人守着，
/// 但 `options` 曾经漏网——`slide_export_dpi` 与 `render_dpi` 就是这样
/// 挂在那里很久的，两个滑杆在 UI 上真实可见、拖动也真有状态，
/// 只是从来没有任何一行 Rust 读过它们。
///
/// 这里的做法是把「已实现的参数键」显式列出来，和注册表对账。
/// 往注册表加参数时这份清单不会自动更新——正是如此它才有用：
/// 加了参数却忘了在 `convert.rs` / `pdf.rs` / `table.rs` / `docx.rs` 里
/// 读它，这个测试会红，而不是等用户发现滑杆没反应。
#[test]
fn every_option_key_is_read_by_the_backend() {
    // 后端 spec.text / spec.num / spec.flag 读过的键（含动作注入的 pdf_op）
    const IMPLEMENTED: &[&str] = &[
        "quality",
        "strip_exif",
        "resize_mode",
        "resize_value",
        "pdf_engine",
        "pdf_op",
        "pdf_mode",
        "pdf_drop",
        "pdf_order",
        "pdf_rotate",
        "split_mode",
        "split_ranges",
        "split_every",
        "page_range",
        "keep_images",
        "sheet_mode",
        "header_row",
        "delimiter",
        "encoding",
        "json_shape",
    ];

    for f in list_formats_json().as_array().unwrap() {
        for o in f["options"].as_array().unwrap() {
            let key = o["key"].as_str().unwrap();
            assert!(
                IMPLEMENTED.contains(&key),
                "参数 {key}（在 {} 上）没有任何后端代码读它——\
                 要么实现它，要么从注册表里删掉。清单在 tests/registry_shape.rs",
                f["id"]
            );
        }
    }
}

/// 注册表里没有「登记了却一定失败」的格式。
///
/// AVIF 曾经是这样：`can_encode: true`、列在每个图片格式的 targets 里，
/// 但 `Cargo.toml` 没开 `image` 的 `avif` feature，所以选中它必然得到
/// 「暂不支持编码为 avif」。SVG / HEIC 同理（解码器没接）。可选却会失败的
/// 条目比没有还糟：用户选了才在转换阶段撞墙。
///
/// 这条守着的是「别再把没实现的格式写进表里」。
#[test]
fn registry_has_no_formats_we_cannot_actually_handle() {
    const NOT_IMPLEMENTED: &[&str] = &["avif", "svg", "heic", "heif", "jxl", "tga", "exr", "dds"];

    let arr = list_formats_json();
    for f in arr.as_array().unwrap() {
        let id = f["id"].as_str().unwrap();
        assert!(
            !NOT_IMPLEMENTED.contains(&id),
            "{id} 没有可用的编解码器，不该出现在注册表里"
        );
        for t in f["targets"].as_array().unwrap() {
            let t = t.as_str().unwrap();
            assert!(
                !NOT_IMPLEMENTED.contains(&t),
                "{} 的 targets 里含有 {t}，但 {t} 没有编码器",
                id
            );
        }
    }
}
