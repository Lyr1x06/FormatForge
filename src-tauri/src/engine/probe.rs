//! Office→PDF 转换引擎探测。
//!
//! 按保真度排序：Microsoft Office COM（1:1） > WPS Office COM（近乎 1:1）
//! > LibreOffice headless（重排版）。
//!
//! 探测本身只读注册表与文件系统，不启动任何 Office 进程——冷启动一个
//! Word 要 2~4 秒，不能在探测阶段就付这个代价。

use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// 避免 `reg.exe` 在 GUI 进程里闪出控制台窗口
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 文档子类型 → 该类型可用的 COM 应用
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DocApp {
    Word,
    Excel,
    Powerpoint,
}

impl DocApp {
    pub fn label(self) -> &'static str {
        match self {
            DocApp::Word => "Word",
            DocApp::Excel => "Excel",
            DocApp::Powerpoint => "PowerPoint",
        }
    }

    /// Microsoft Office 的 ProgID
    fn office_progid(self) -> &'static str {
        match self {
            DocApp::Word => "Word.Application",
            DocApp::Excel => "Excel.Application",
            DocApp::Powerpoint => "PowerPoint.Application",
        }
    }

    /// WPS Office 的 ProgID（文字/表格/演示）
    fn wps_progid(self) -> &'static str {
        match self {
            DocApp::Word => "KWPS.Application",
            DocApp::Excel => "KET.Application",
            DocApp::Powerpoint => "KWPP.Application",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineReport {
    pub id: String,
    pub label: String,
    pub available: bool,
    /// "exact" | "near" | "relayout"
    pub fidelity: String,
    pub fidelity_note: String,
    /// 该引擎当前可处理的文档类型
    pub apps: Vec<DocApp>,
    /// 人类可读的探测细节，直接展示给用户
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub engines: Vec<EngineReport>,
    /// 推荐引擎 id；全都不可用时为 None
    pub recommended: Option<String>,
    /// LibreOffice 安装引导
    pub libreoffice_winget_id: String,
}

/// 探测全部引擎。前端启动时调一次。
///
/// 命令包装在 `commands` 里；这里保持纯函数，方便直接测试。
pub fn probe_all() -> ProbeResult {
    let engines = vec![probe_office(), probe_wps(), probe_libreoffice()];
    let recommended = engines
        .iter()
        .find(|e| e.available)
        .map(|e| e.id.clone());

    ProbeResult {
        engines,
        recommended,
        libreoffice_winget_id: "TheDocumentFoundation.LibreOffice".into(),
    }
}

fn probe_office() -> EngineReport {
    let apps: Vec<DocApp> = [DocApp::Word, DocApp::Excel, DocApp::Powerpoint]
        .into_iter()
        .filter(|a| progid_registered(a.office_progid()))
        .collect();

    let available = !apps.is_empty();
    let detail = if available {
        let names: Vec<&str> = apps.iter().map(|a| a.label()).collect();
        format!("已检测到 {}", names.join(" / "))
    } else if office_installed_but_unregistered() {
        "检测到 Office 安装目录，但 COM 组件未注册——可能需要以管理员身份运行一次 Office 修复".into()
    } else {
        "未检测到 Microsoft Office".into()
    };

    EngineReport {
        id: "office".into(),
        label: "Microsoft Office".into(),
        available,
        fidelity: "exact".into(),
        fidelity_note: "与 Office 中所见完全一致".into(),
        apps,
        detail,
    }
}

fn probe_wps() -> EngineReport {
    let apps: Vec<DocApp> = [DocApp::Word, DocApp::Excel, DocApp::Powerpoint]
        .into_iter()
        .filter(|a| progid_registered(a.wps_progid()))
        .collect();

    let available = !apps.is_empty();
    let detail = if available {
        let names: Vec<&str> = apps.iter().map(|a| a.label()).collect();
        format!("已检测到 WPS {}", names.join(" / "))
    } else {
        "未检测到 WPS Office".into()
    };

    EngineReport {
        id: "wps".into(),
        label: "WPS Office".into(),
        available,
        fidelity: "near".into(),
        fidelity_note: "与 Office 高度接近，极少数复杂排版可能有细微差异".into(),
        apps,
        detail,
    }
}

fn probe_libreoffice() -> EngineReport {
    let found = soffice_path();
    let available = found.is_some();
    let detail = match &found {
        Some(p) => format!("已检测到 {}", p.display()),
        None => "未安装，可通过 winget 一键安装".into(),
    };

    EngineReport {
        id: "libreoffice".into(),
        label: "LibreOffice".into(),
        available,
        fidelity: "relayout".into(),
        fidelity_note: "免费方案，会重新排版，复杂文档的换行与分页可能与 Office 不同".into(),
        // LibreOffice 走无头转换，三种文档类型都能处理
        apps: if available {
            vec![DocApp::Word, DocApp::Excel, DocApp::Powerpoint]
        } else {
            vec![]
        },
        detail,
    }
}

/// 查 HKCR\<ProgID>\CLSID 是否存在且指向一个 GUID。
fn progid_registered(progid: &str) -> bool {
    #[cfg(not(windows))]
    {
        let _ = progid;
        return false;
    }

    #[cfg(windows)]
    {
        let out = Command::new("reg")
            .args(["query", &format!("HKCR\\{progid}\\CLSID"), "/ve"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        let Ok(out) = out else { return false };
        if !out.status.success() {
            return false;
        }
        // 输出是控制台代码页编码，但我们要找的 ProgID 与 GUID 都是 ASCII，
        // lossy 解码后做子串匹配是安全的。
        let text = String::from_utf8_lossy(&out.stdout);
        text.contains("REG_SZ") && text.contains('{')
    }
}

/// Office 装了但 COM 没注册时的兜底提示。
fn office_installed_but_unregistered() -> bool {
    office_root_candidates().into_iter().any(|p| p.is_dir())
}

fn office_root_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for base in ["C:\\Program Files\\Microsoft Office\\root", "D:\\Program Files\\Microsoft Office\\root"] {
        v.push(PathBuf::from(base));
    }
    v
}

/// 找 soffice.exe。覆盖默认安装路径与 PATH。
pub fn soffice_path() -> Option<PathBuf> {
    let candidates = [
        "C:\\Program Files\\LibreOffice\\program\\soffice.exe",
        "C:\\Program Files (x86)\\LibreOffice\\program\\soffice.exe",
        "D:\\Program Files\\LibreOffice\\program\\soffice.exe",
        "D:\\Program Files (x86)\\LibreOffice\\program\\soffice.exe",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local).join("Programs\\LibreOffice\\program\\soffice.exe");
        if p.is_file() {
            return Some(p);
        }
    }
    which_soffice()
}

fn which_soffice() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("soffice.exe"))
        .find(|p| p.is_file())
}

/// 用系统默认程序打开 URL（LibreOffice 安装引导用）。
#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    // 只允许 https，避免被前端误用成任意命令执行
    if !url.starts_with("https://") {
        return Err("只允许打开 https 链接".into());
    }
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &url])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err("仅支持 Windows".into())
    }
}
