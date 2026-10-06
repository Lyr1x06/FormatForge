//! 单文件转换实现。
//!
//! 每个转换都是「读源 → 转 → 写目标」的原子操作：产物先写成 `.fftmp`
//! 临时名，全部成功后再改名到最终路径。中途取消或崩溃都不会在输出目录里
//! 留下半成品文件。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::engine::{FailureKind, OfficeApp, OfficePool};
use crate::formats::Category;
use crate::job::{job_error, simple_error, CancelToken, JobSpec, JobState, Reporter};
use crate::paths;

/// 本地泳道入口：图片、表格数据、文档文本提取、PDF 操作。
pub fn run_local(spec: &JobSpec, _out_dir: &Path, cancel: &CancelToken, reporter: &Reporter) {
    if cancel.is_cancelled() {
        reporter.state(spec.id, JobState::Cancelled, None);
        return;
    }
    reporter.state(spec.id, JobState::Preparing, None);

    // PDF 源统一走 PDF 模块。注意 xlsx/docx → pdf 不在这里——那些在
    // office 泳道由 job.rs 分流，根本不会进到这个函数。
    if spec.src_format == "pdf" || spec.src_format == crate::pdf::MERGE_SRC {
        crate::pdf::run(spec, cancel, reporter);
        return;
    }

    // 按**目标**分派而不是源：同一个 xlsx 转 csv 走表格管线，
    // 转 pdf 就得走 Office 管线（那条在 office 泳道里）。
    match spec.dst_format.as_str() {
        "csv" | "tsv" | "json" | "xlsx" => crate::table::run(spec, cancel, reporter),
        "txt" | "md" | "html" if spec.src_format == "docx" => {
            crate::docx::run(spec, cancel, reporter)
        }
        _ => match Category::of(&spec.src_format) {
            Category::Image => run_image(spec, cancel, reporter),
            cat => reporter.state(
                spec.id,
                JobState::Failed,
                Some(simple_error(
                    "unsupported",
                    format!(
                        "暂不支持 {:?} 类别下的 {} → {}",
                        cat, spec.src_format, spec.dst_format
                    ),
                )),
            ),
        },
    }
}

/// 图片转换。目前走 `image` crate 的通用编解码路径。
fn run_image(spec: &JobSpec, cancel: &CancelToken, reporter: &Reporter) {
    use image::ImageReader;

    let src = PathBuf::from(&spec.src);
    let dst = PathBuf::from(&spec.dst);
    let bytes_in = file_len(&src);

    if cancel.is_cancelled() {
        reporter.state(spec.id, JobState::Cancelled, None);
        return;
    }

    reporter.state(spec.id, JobState::Converting, None);
    reporter.progress(spec.id, 0.1);

    let decoded = ImageReader::open(paths::for_io(&src)).and_then(|r| r.decode().map_err(std::io::Error::other));
    let img = match decoded {
        Ok(i) => i,
        Err(e) => {
            // io::Error 的 source 里可能藏着真正的解码错误，尽量还原
            let msg = match e.get_ref() {
                Some(inner) => inner.to_string(),
                None => e.to_string(),
            };
            let kind = match e.kind() {
                std::io::ErrorKind::NotFound => "not_found",
                std::io::ErrorKind::PermissionDenied => "access_denied",
                _ => "corrupt",
            };
            reporter.state(spec.id, JobState::Failed, Some(simple_error(kind, msg)));
            return;
        }
    };

    reporter.progress(spec.id, 0.5);

    if cancel.is_cancelled() {
        reporter.state(spec.id, JobState::Cancelled, None);
        return;
    }

    // ---- 参数 ----
    let img = apply_resize(img, spec);
    let quality = spec.num("quality", 85.0) as u8;

    let fmt = match image_format(&spec.dst_format) {
        Some(f) => f,
        None => {
            reporter.state(
                spec.id,
                JobState::Failed,
                Some(simple_error("unsupported", format!("暂不支持编码为 {}", spec.dst_format))),
            );
            return;
        }
    };

    // 写临时名，成功后原子改名
    let tmp = temp_sibling(&dst);
    if let Err(e) = encode(&img, &tmp, fmt, quality) {
        let _ = std::fs::remove_file(paths::for_io(&tmp));
        reporter.state(
            spec.id,
            JobState::Failed,
            Some(simple_error("encode", format!("写入失败：{e}"))),
        );
        return;
    }

    // EXIF 剥离：image crate 的通用编码器本来就不写 EXIF，
    // 所以这里只在需要保留时才是额外工作——当前一律剥离，无需额外处理。
    let _ = spec.flag("strip_exif", true);

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
    reporter.output(spec.id, &spec.dst, bytes_in, file_len(&dst), 0);
    reporter.state(spec.id, JobState::Done, None);
}

/// Office 文档 → PDF，走 COM 拿 1:1 保真。
pub fn run_office_pdf(
    spec: &JobSpec,
    _out_dir: &Path,
    cancel: &CancelToken,
    reporter: &Reporter,
    pool: &Arc<Mutex<OfficePool>>,
) {
    let Some(app) = OfficeApp::from_ext(&spec.src_format) else {
        reporter.state(
            spec.id,
            JobState::Failed,
            Some(simple_error("unsupported", format!("{} 不是 Office 文档", spec.src_format))),
        );
        return;
    };

    // 引擎选择。目前只实现了 Office COM 一条路；显式选了别的要如实拒绝，
    // 而不是偷偷用 Office 转完再假装是 LibreOffice 的结果。
    //
    // 注意这里对 libeoffice 也**无条件**拒绝，不能只在「探测不到 soffice」时
    // 拒绝：装了 LibreOffice 的机器上会顺着往下走，最后落到 OfficePool 里
    // 用 Word 转出 PDF —— 那正是「偷偷换个引擎还谎报」。
    let engine = spec.text("pdf_engine", "auto");
    match engine {
        "wps" => {
            reporter.state(
                spec.id,
                JobState::Failed,
                Some(simple_error("unsupported", "WPS 引擎尚未接入，请改用「自动选择」")),
            );
            return;
        }
        "libreoffice" => {
            reporter.state(
                spec.id,
                JobState::Failed,
                Some(simple_error(
                    "unsupported",
                    "LibreOffice 引擎尚未接入，请改用「自动选择」",
                )),
            );
            return;
        }
        _ => {}
    }

    if cancel.is_cancelled() {
        reporter.state(spec.id, JobState::Cancelled, None);
        return;
    }

    let src = PathBuf::from(&spec.src);
    let dst = PathBuf::from(&spec.dst);
    let bytes_in = file_len(&src);
    let tmp = temp_sibling(&dst);

    // Word/Excel 内部的 CreateFile 不认 \\?\ 前缀，路径过长会失败得莫名其妙。
    // 超过安全长度就搬到短路径下转换，再把产物搬回来。
    let staging = match Staging::prepare(&src, &tmp) {
        Ok(s) => s,
        Err(e) => {
            reporter.state(spec.id, JobState::Failed, Some(simple_error("io", e)));
            return;
        }
    };

    reporter.state(spec.id, JobState::Preparing, None);

    let outcome: Result<u64, (FailureKind, String)> = {
        let mut guard = match pool.lock() {
            Ok(g) => g,
            Err(_) => {
                reporter.state(
                    spec.id,
                    JobState::Failed,
                    Some(simple_error("internal", "引擎池状态异常")),
                );
                return;
            }
        };

        match guard.worker(app) {
            Ok(worker) => {
                reporter.state(spec.id, JobState::Converting, None);
                reporter.progress(spec.id, 0.15);
                match worker.convert(
                    &staging.convert_src.to_string_lossy(),
                    &staging.convert_dst.to_string_lossy(),
                ) {
                    Ok(ms) => Ok(ms),
                    Err((kind, detail)) => {
                        // 超时/崩溃是瞬时故障，丢掉 worker 让下个文件重建。
                        // 「没装 Office」是硬失败，由 pool 缓存住避免反复付超时代价。
                        if matches!(kind, FailureKind::Timeout | FailureKind::Crashed) {
                            guard.discard(app);
                        }
                        Err((kind, detail))
                    }
                }
            }
            Err(reason) => {
                guard.note_hard_failure(app, reason.clone());
                Err((FailureKind::NotInstalled, reason))
            }
        }
    };

    let result = staging.collect(&tmp, &dst);
    staging.cleanup();

    match outcome {
        Ok(_) => match result {
            Ok(()) => {
                reporter.progress(spec.id, 1.0);
                reporter.output(spec.id, &spec.dst, bytes_in, file_len(&dst), 0);
                reporter.state(spec.id, JobState::Done, None);
            }
            Err(e) => reporter.state(spec.id, JobState::Failed, Some(simple_error("io", e))),
        },
        Err((kind, detail)) => {
            // 转换失败时清掉可能残留的临时产物
            let _ = std::fs::remove_file(paths::for_io(&tmp));
            reporter.state(spec.id, JobState::Failed, Some(job_error(kind, app, detail)));
        }
    }
}

/* ---------------------------------------------------------------- 暂存 */

/// Office 路径安全长度。留了余量给可能的 ` (1)` 后缀。
const OFFICE_PATH_SAFE: usize = 235;

struct Staging {
    convert_src: PathBuf,
    convert_dst: PathBuf,
    /// 需要把产物搬回的真实目标；不暂存时为 None
    real_dst: Option<PathBuf>,
    temp_dir: Option<PathBuf>,
}

impl Staging {
    fn prepare(src: &Path, tmp_dst: &Path) -> Result<Self, String> {
        let long = src.to_string_lossy().len() > OFFICE_PATH_SAFE
            || tmp_dst.to_string_lossy().len() > OFFICE_PATH_SAFE;

        if !long {
            return Ok(Staging {
                convert_src: src.to_path_buf(),
                convert_dst: tmp_dst.to_path_buf(),
                real_dst: None,
                temp_dir: None,
            });
        }

        let dir = std::env::temp_dir().join(format!("format-forge-{}", std::process::id()));
        std::fs::create_dir_all(paths::for_io(&dir))
            .map_err(|e| format!("无法创建暂存目录：{e}"))?;

        let tag = short_hash(&src.to_string_lossy());
        let staged_src = dir.join(format!("{tag}.{}", ext_of(src)));
        std::fs::copy(paths::for_io(src), paths::for_io(&staged_src))
            .map_err(|e| format!("暂存源文件失败：{e}"))?;

        Ok(Staging {
            convert_src: staged_src,
            convert_dst: dir.join(format!("{tag}.pdf")),
            real_dst: Some(tmp_dst.to_path_buf()),
            temp_dir: Some(dir),
        })
    }

    /// 把产物放到 `tmp_dst`，然后原子改名到 `final_dst`。
    fn collect(&self, tmp_dst: &Path, final_dst: &Path) -> Result<(), String> {
        if !self.convert_dst.exists() {
            return Err("转换结束但找不到输出文件".into());
        }
        // 暂存模式下产物在别处，先复制到 tmp_dst 再改名，保证最终一步是原子的
        if self.real_dst.is_some() {
            std::fs::copy(paths::for_io(&self.convert_dst), paths::for_io(tmp_dst))
                .map_err(|e| format!("写回输出文件失败：{e}"))?;
        }
        std::fs::rename(paths::for_io(tmp_dst), paths::for_io(final_dst))
            .map_err(|e| format!("无法落地输出文件：{e}"))
    }

    fn cleanup(&self) {
        if let Some(dir) = &self.temp_dir {
            let _ = std::fs::remove_dir_all(paths::for_io(dir));
        }
    }
}

/* ---------------------------------------------------------------- 小工具 */

/// 按参数缩放。`image` 的 `resize` 用的是 Lanczos3，画质够用。
fn apply_resize(img: image::DynamicImage, spec: &JobSpec) -> image::DynamicImage {
    let mode = spec.text("resize_mode", "none");
    if mode == "none" {
        return img;
    }
    let target = spec.num("resize_value", 1600.0).max(1.0) as u32;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return img;
    }

    let (nw, nh) = match mode {
        // 只缩不放：把已小于目标的图片放大只会变糊
        "longest" => {
            if w.max(h) <= target {
                return img;
            }
            if w >= h {
                (target, (h as f64 * target as f64 / w as f64).round() as u32)
            } else {
                ((w as f64 * target as f64 / h as f64).round() as u32, target)
            }
        }
        "width" => {
            if w <= target {
                return img;
            }
            (target, (h as f64 * target as f64 / w as f64).round() as u32)
        }
        "height" => {
            if h <= target {
                return img;
            }
            ((w as f64 * target as f64 / h as f64).round() as u32, target)
        }
        _ => return img,
    };

    img.resize_exact(nw.max(1), nh.max(1), image::imageops::FilterType::Lanczos3)
}

/// 按目标格式编码。
///
/// JPEG 单独走 `JpegEncoder` 才能把质量传下去——`save_with_format` 用的是
/// 默认质量 75，用户拖滑杆会毫无反应。
fn encode(
    img: &image::DynamicImage,
    path: &Path,
    fmt: image::ImageFormat,
    quality: u8,
) -> Result<(), image::ImageError> {
    use std::fs::File;
    use std::io::BufWriter;

    let p = paths::for_io(path);
    match fmt {
        image::ImageFormat::Jpeg => {
            let file = File::create(&p)?;
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(
                BufWriter::new(file),
                quality,
            );
            // JPEG 不支持透明，带 alpha 的图先转成 RGB8 再编码
            let rgb = img.to_rgb8();
            enc.encode(&rgb, rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)?;
            Ok(())
        }
        other => img.save_with_format(&p, other),
    }
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(paths::for_io(p)).map(|m| m.len()).unwrap_or(0)
}

/// 同目录下的临时文件名。放同目录是为了让 rename 落在同一个卷上。
fn temp_sibling(dst: &Path) -> PathBuf {
    let ext = dst.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let name = dst.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    dst.with_file_name(format!("{name}.{ext}.fftmp"))
}

fn image_format(id: &str) -> Option<image::ImageFormat> {
    Some(match id {
        "png" => image::ImageFormat::Png,
        "jpeg" => image::ImageFormat::Jpeg,
        "webp" => image::ImageFormat::WebP,
        "gif" => image::ImageFormat::Gif,
        "bmp" => image::ImageFormat::Bmp,
        "tiff" => image::ImageFormat::Tiff,
        "ico" => image::ImageFormat::Ico,
        _ => return None,
    })
}

fn ext_of(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| "bin".into())
}

fn short_hash(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:06x}", h.finish() & 0xffffff)
}
