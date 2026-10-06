//! 路径工具。
//!
//! Windows 的 `MAX_PATH` 是 260；超过就得用 `\\?\` 前缀走长路径 API。
//! 但 `\\?\` 绝对不能被用户看到，也不能传给 Office（Word/Excel 内部用的是
//! 不带前缀的 CreateFile，收到 `\\?\` 路径会直接失败）。
//!
//! 所以规则是：内部存储一律用干净路径，只在真正调用文件系统的那一刻加前缀，
//! 任何要展示、要传给 COM 的地方都用干净路径。

use std::path::{Path, PathBuf};

/// Windows 传统路径上限
const MAX_PATH: usize = 260;
/// 留出余量，因为输出文件名可能还要加 ` (1)` 后缀或 `.part` 临时后缀
pub const PATH_BUDGET: usize = 240;

/// 为文件系统调用准备的路径：超长时加 `\\?\` 前缀。
///
/// 只在长度确实会超限时才加，因为加了前缀的路径会绕过路径规范化，
/// 相对路径和 `/` 分隔符都会失效。
pub fn for_io(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if s.len() < MAX_PATH {
        return p.to_path_buf();
    }
    #[cfg(windows)]
    {
        // UNC 路径要写成 \\?\UNC\server\share
        if let Some(rest) = s.strip_prefix(r"\\") {
            return PathBuf::from(format!(r"\\?\UNC\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(format!(r"\\?\{rest}"));
        }
        // 相对路径无法直接加前缀，先绝对化
        if p.is_absolute() {
            PathBuf::from(format!(r"\\?\{s}"))
        } else if let Ok(abs) = std::path::absolute(p) {
            PathBuf::from(format!(r"\\?\{abs}", abs = abs.display()))
        } else {
            p.to_path_buf()
        }
    }
    #[cfg(not(windows))]
    {
        p.to_path_buf()
    }
}

/// 展示用路径：剥掉 `\\?\` 前缀，浏览器与资源管理器都认干净的那个。
pub fn for_display(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    s
}

/// 路径长度是否已经逼近上限。返回超出预算的字节数（0 表示安全）。
pub fn over_budget(p: &Path) -> usize {
    let len = p.to_string_lossy().len();
    len.saturating_sub(PATH_BUDGET)
}

/// 拆出文件名与扩展名。扩展名统一小写，无扩展名时为空串。
pub fn split_ext(p: &Path) -> (String, String) {
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    (stem, ext)
}

/// 输出目录里选一个不冲突的文件名。
///
/// 两个冲突来源都要考虑：磁盘上已存在的文件，以及**本批已经规划过的名字**。
/// 后者是必需的——`photo.png` 和 `photo.jpg` 同时转 WebP 会撞同一个输出路径，
/// 光看磁盘是发现不了的。
///
/// `claimed` 存放已占用的名字（小写）。
pub fn pick_free_name(
    dir: &Path,
    stem: &str,
    ext: &str,
    claimed: &mut std::collections::HashSet<String>,
) -> String {
    // 源文件本身可能已经带 (2) 这类后缀，先剥掉再枚举，
    // 否则会产出 "photo (2) (2).webp"
    let base = strip_copy_suffix(stem);

    let take = |claimed: &mut std::collections::HashSet<String>, name: String| -> String {
        claimed.insert(name.to_ascii_lowercase());
        name
    };

    let candidate = format!("{base}.{ext}");
    if !claimed.contains(&candidate.to_ascii_lowercase()) && !dir.join(&candidate).exists() {
        return take(claimed, candidate);
    }

    for n in 2..=9999u32 {
        let c = format!("{base} ({n}).{ext}");
        if !claimed.contains(&c.to_ascii_lowercase()) && !dir.join(&c).exists() {
            return take(claimed, c);
        }
    }

    // 名字用尽就退回带时间戳的，总比失败好
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    take(claimed, format!("{base}-{ts}.{ext}"))
}

/// 剥掉 "name (2)" 里的 " (2)"
fn strip_copy_suffix(stem: &str) -> String {
    let trimmed = stem.trim_end();
    if !trimmed.ends_with(')') {
        return stem.to_string();
    }
    let Some(open) = trimmed.rfind(" (") else {
        return stem.to_string();
    };
    let inner = &trimmed[open + 2..trimmed.len() - 1];
    if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_digit()) {
        return stem.to_string();
    }
    trimmed[..open].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_copy_suffix() {
        assert_eq!(strip_copy_suffix("photo"), "photo");
        assert_eq!(strip_copy_suffix("photo (2)"), "photo");
        assert_eq!(strip_copy_suffix("photo (12)"), "photo");
        assert_eq!(strip_copy_suffix("photo (a)"), "photo (a)");
        assert_eq!(strip_copy_suffix("my (holiday) photo"), "my (holiday) photo");
    }

    #[test]
    fn splits_ext_lowercased() {
        let (s, e) = split_ext(Path::new("C:/a/b/Photo.JPG"));
        assert_eq!(s, "Photo");
        assert_eq!(e, "jpg");
    }

    #[test]
    fn display_strips_prefix() {
        assert_eq!(for_display(Path::new(r"\\?\C:\a\b")), r"C:\a\b");
        assert_eq!(for_display(Path::new(r"\\?\UNC\srv\share\a")), r"\\srv\share\a");
        assert_eq!(for_display(Path::new(r"C:\a\b")), r"C:\a\b");
    }
}
