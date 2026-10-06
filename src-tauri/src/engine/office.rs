//! 常驻 Office COM worker 的 Rust 侧管理。
//!
//! 每个 Office 应用（Word / Excel / PowerPoint）惰性启动一个 PowerShell 子进程，
//! 进程内持有一个 Application COM 实例并全程复用。Word 冷启动 2~4 秒，
//! 逐文件重建会让批量慢一个数量级。
//!
//! 通信协议：stdin/stdout 上的行分隔 JSON，所有协议行带 `@@FF@@` 前缀，
//! 不带前缀的一律当噪声丢弃（Office 会往 stdout 写字）。
//!
//! 实测确认的两个坑（都会以极具误导性的报错出现）：
//!   * Word 的可选参数必须传 `[Type]::Missing`，传 `''` 会得到
//!     「这是一个无效文件名」，看起来像路径问题，实际不是。
//!   * Excel 同样必须传 `[Type]::Missing`，传 `$null` 会得到
//!     「不能取得类 Workbooks 的 Open 属性」，看起来像权限问题，实际不是。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// COM 冷启动预算。超出即认为机器上有模态框卡着。
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
/// 单文件转换预算。含 Word 打开 + 导出 + 关闭。
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

const PROTO_PREFIX: &str = "@@FF@@";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OfficeApp {
    Word,
    Excel,
    Powerpoint,
}

impl OfficeApp {
    pub fn id(self) -> &'static str {
        match self {
            OfficeApp::Word => "word",
            OfficeApp::Excel => "excel",
            OfficeApp::Powerpoint => "powerpoint",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            OfficeApp::Word => "Word",
            OfficeApp::Excel => "Excel",
            OfficeApp::Powerpoint => "PowerPoint",
        }
    }

    /// 该应用负责的源文件扩展名
    pub fn source_exts(self) -> &'static [&'static str] {
        match self {
            OfficeApp::Word => &["docx", "docm", "dotx", "dotm", "doc", "rtf", "odt"],
            OfficeApp::Excel => &["xlsx", "xlsm", "xltx", "xltm", "xls", "ods"],
            OfficeApp::Powerpoint => &["pptx", "pptm", "potx", "potm", "ppt", "odp"],
        }
    }

    pub fn from_ext(ext: &str) -> Option<Self> {
        let e = ext.to_ascii_lowercase();
        [OfficeApp::Word, OfficeApp::Excel, OfficeApp::Powerpoint]
            .into_iter()
            .find(|a| a.source_exts().contains(&e.as_str()))
    }
}

/// 失败归类。Rust 侧做这件事而不是 PowerShell，因为 PS 脚本要保持纯 ASCII
/// （Windows PowerShell 5.1 按系统 ANSI 码页读 .ps1，中文源码会被破坏）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Password,
    Corrupt,
    Locked,
    NotFound,
    AccessDenied,
    NotInstalled,
    Timeout,
    Crashed,
    Unknown,
}

impl FailureKind {
    /// 稳定的机器可读标识，前端据此选图标与颜色
    pub fn code(self) -> &'static str {
        match self {
            FailureKind::Password => "password",
            FailureKind::Corrupt => "corrupt",
            FailureKind::Locked => "locked",
            FailureKind::NotFound => "not_found",
            FailureKind::AccessDenied => "access_denied",
            FailureKind::NotInstalled => "not_installed",
            FailureKind::Timeout => "timeout",
            FailureKind::Crashed => "crashed",
            FailureKind::Unknown => "unknown",
        }
    }

    /// 面向用户的说明。COM 原始消息经常是本地化的、且指向错误的方向
    /// （例如「这是一个无效文件名」其实与文件名无关），所以这里自己写。
    pub fn user_message(self, app: OfficeApp) -> String {
        match self {
            FailureKind::Password => "文档已加密，需要密码才能转换".into(),
            FailureKind::Corrupt => "文件已损坏或不是有效的 Office 文档".into(),
            FailureKind::Locked => "文件被其他程序占用，请关闭后重试".into(),
            FailureKind::NotFound => "找不到源文件".into(),
            FailureKind::AccessDenied => "没有权限读写该文件".into(),
            FailureKind::NotInstalled => format!("未安装 {}，无法使用 1:1 保真引擎", app.label()),
            FailureKind::Timeout => format!(
                "{} 长时间无响应，已终止本次转换。可能有对话框被卡住，或文档含大量公式/外部链接",
                app.label()
            ),
            FailureKind::Crashed => format!("{} 进程异常退出，已自动重启", app.label()),
            FailureKind::Unknown => "转换失败".into(),
        }
    }
}

/// 把 COM 的 HRESULT 与消息映射成分类。
///
/// 消息可能是中文（本地化 Office），但**不要按中文关键字匹配**——那个方向
/// 已经被证实会误判：「这是一个无效文件名」实际原因是可选参数传了空串。
/// 所以这里只认 HRESULT 这类确定性信号，其余交给消息里的英文关键字。
pub fn classify(hresult: i32, msg: &str, inner: &str) -> FailureKind {
    let text = format!("{msg} {inner}");
    let lower = text.to_ascii_lowercase();

    // Windows 错误码
    match hresult as u32 {
        0x8007_0002 => return FailureKind::NotFound,   // ERROR_FILE_NOT_FOUND
        0x8007_0005 => return FailureKind::AccessDenied,
        0x8003_00D5 => return FailureKind::Password,   // Word: 文件已加密
        0x8004_0154 => return FailureKind::Password,   // Excel: 密码
        0x8004_015B => return FailureKind::Corrupt,    // Excel: 文件格式无效
        0x8000_4005 => {}                              // E_FAIL：信息不足，靠消息判断
        _ => {}
    }

    if lower.contains("password") || lower.contains("encrypted") {
        return FailureKind::Password;
    }
    if lower.contains("corrupt")
        || lower.contains("damaged")
        || lower.contains("not a valid")
        || lower.contains("unreadable")
    {
        return FailureKind::Corrupt;
    }
    if lower.contains("being used by another")
        || lower.contains("locked for editing")
        || lower.contains("read-only")
    {
        return FailureKind::Locked;
    }
    if lower.contains("class not registered") || hresult as u32 == 0x8004_0154 {
        return FailureKind::NotInstalled;
    }
    if lower.contains("access is denied") {
        return FailureKind::AccessDenied;
    }

    // 本地化消息的兜底：Office 的中文报错里这几类有稳定的措辞
    if msg.contains("密码") || msg.contains("加密") {
        return FailureKind::Password;
    }
    if msg.contains("损坏") {
        return FailureKind::Corrupt;
    }
    if msg.contains("占用") || msg.contains("锁定") {
        return FailureKind::Locked;
    }

    FailureKind::Unknown
}

/// worker 回传的一条消息
#[derive(Debug, Clone)]
enum Reply {
    Ready,
    Pong,
    Ok { ms: u64 },
    Err { hresult: i32, msg: String, inner: String },
    Fatal { hresult: i32, msg: String },
}

/// 一个常驻的 Office worker
pub struct OfficeWorker {
    app: OfficeApp,
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Reply>,
    /// 线程是否还活着；false 表示 stdout 已到 EOF，进程死了
    alive: bool,
}

impl OfficeWorker {
    /// 启动 worker 并等待 `ready`。冷启动就在这里付掉。
    pub fn start(app: OfficeApp, script: PathBuf) -> Result<Self, String> {
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .args(["-App", app.id()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("无法启动 PowerShell：{e}"))?;

        let stdin = child.stdin.take().ok_or("无法获取 worker stdin")?;
        let stdout = child.stdout.take().ok_or("无法获取 worker stdout")?;

        // stderr 单独抽干，避免管道写满导致子进程阻塞
        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    eprintln!("[office-worker] {line}");
                }
            });
        }

        let rx = spawn_reader(stdout);

        let mut worker = OfficeWorker { app, child, stdin, rx, alive: true };

        // 等 ready。这一步超时通常意味着 Office 弹了模态框在等人点。
        match worker.rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Reply::Ready) => Ok(worker),
            Ok(Reply::Fatal { hresult, msg }) => {
                worker.kill();
                Err(format!(
                    "{} 启动失败：{}（{}）",
                    app.label(),
                    describe_start_failure(hresult, &msg).code(),
                    msg
                ))
            }
            Ok(other) => {
                worker.kill();
                Err(format!("{} 启动返回了意外消息：{other:?}", app.label()))
            }
            Err(RecvTimeoutError::Timeout) => {
                worker.kill();
                Err(format!(
                    "{} 启动超时（{} 秒）。常见原因：Office 正在弹出一个对话框等待点击，\
                     或首次启动需要完成初始化。请手动打开一次 {} 并关闭，然后重试",
                    app.label(),
                    STARTUP_TIMEOUT.as_secs(),
                    app.label()
                ))
            }
            Err(RecvTimeoutError::Disconnected) => {
                worker.kill();
                Err(format!("{} 进程意外退出", app.label()))
            }
        }
    }

    /// 转换一个文件。阻塞直到出结果或超时。
    pub fn convert(&mut self, src: &str, dst: &str) -> Result<u64, (FailureKind, String)> {
        if !self.alive {
            return Err((FailureKind::Crashed, "worker 已失效".into()));
        }

        let req = serde_json::json!({ "op": "convert", "src": src, "dst": dst });
        self.send(&req)
            .map_err(|e| (FailureKind::Crashed, format!("写入 worker 失败：{e}")))?;

        let deadline = Instant::now() + JOB_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // 同步 COM 调用无法从外部打断，只能杀掉进程
                self.kill();
                return Err((
                    FailureKind::Timeout,
                    self.app.label().to_string(),
                ));
            }
            match self.rx.recv_timeout(remaining) {
                Ok(Reply::Ok { ms, .. }) => return Ok(ms),
                Ok(Reply::Err { hresult, msg, inner }) => {
                    return Err((classify(hresult, &msg, &inner), msg));
                }
                Ok(Reply::Fatal { hresult, msg }) => {
                    self.kill();
                    return Err((describe_start_failure(hresult, &msg), msg));
                }
                // ready / pong 出现在这里说明协议错位，忽略继续等
                Ok(Reply::Ready) | Ok(Reply::Pong) => continue,
                Err(RecvTimeoutError::Timeout) => {
                    self.kill();
                    return Err((
                        FailureKind::Timeout,
                        format!("{} 秒无响应", JOB_TIMEOUT.as_secs()),
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.alive = false;
                    return Err((FailureKind::Crashed, "worker 进程已退出".into()));
                }
            }
        }
    }

    fn send(&mut self, value: &serde_json::Value) -> std::io::Result<()> {
        let mut line = serde_json::to_string(value)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()
    }

    /// 收尾：先关机指令（让 worker 自己 Quit + 清理），超时就强杀。
    pub fn shutdown(&mut self) {
        if !self.alive {
            self.kill();
            return;
        }
        let _ = self.send(&serde_json::json!({ "op": "quit" }));
        let deadline = Instant::now() + Duration::from_secs(25);
        while Instant::now() < deadline {
            match self.rx.recv_timeout(Duration::from_millis(250)) {
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {
                    // worker 正常退出时 stdout 关闭，try_wait 会给出结果
                    if matches!(self.child.try_wait(), Ok(Some(_))) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        self.kill();
    }

    /// 强杀。worker 脚本自己也会在退出时清理它启动的 Office 进程，
    /// 但被强杀时没有机会执行，所以这里补一遍兜底。
    pub fn kill(&mut self) {
        self.alive = false;
        let _ = self.child.kill();
        let _ = self.child.wait();
        reap_orphan(self.app);
    }
}

impl Drop for OfficeWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 兜底清理：杀掉本 worker 启动的 Office 进程。
///
/// 判据是启动时间——我们要求 worker 启动前该应用必须处于关闭状态，
/// 所以「启动时间晚于 worker」的进程一定是我们的。
fn reap_orphan(app: OfficeApp) {
    let name = match app {
        OfficeApp::Word => "WINWORD",
        OfficeApp::Excel => "EXCEL",
        OfficeApp::Powerpoint => "POWERPNT",
    };
    #[cfg(windows)]
    {
        // PowerShell 自身退出后 Office 可能仍在关闭中，稍等一下
        thread::sleep(Duration::from_millis(400));
        let status = Command::new("taskkill")
            .args(["/F", "/IM", &format!("{name}.EXE"), "/FI", &format!("PID gt 0")])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = status;
    }
    #[cfg(not(windows))]
    {
        let _ = name;
    }
}

fn describe_start_failure(hresult: i32, msg: &str) -> FailureKind {
    let lower = msg.to_ascii_lowercase();
    if lower.contains("class not registered") || hresult as u32 == 0x8004_0154 {
        return FailureKind::NotInstalled;
    }
    if lower.contains("cannot create") || lower.contains("找不到") && lower.contains("应用") {
        return FailureKind::NotInstalled;
    }
    FailureKind::Crashed
}

/// 抽干 worker 的 stdout，逐行解析带前缀的协议行，投递到 channel。
fn spawn_reader(stdout: ChildStdout) -> Receiver<Reply> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some(body) = trimmed.strip_prefix(PROTO_PREFIX) else {
                // 不带前缀 = Office 或 PowerShell 的杂音，丢弃
                continue;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
                continue;
            };
            let reply = match v.get("op").and_then(|o| o.as_str()) {
                Some("ready") => Reply::Ready,
                Some("pong") => Reply::Pong,
                Some("ok") => Reply::Ok {
                    ms: v.get("ms").and_then(|m| m.as_u64()).unwrap_or(0),
                },
                Some("err") => Reply::Err {
                    hresult: v.get("hresult").and_then(|h| h.as_i64()).unwrap_or(0) as i32,
                    msg: v.get("msg").and_then(|m| m.as_str()).unwrap_or("").to_string(),
                    inner: v.get("inner").and_then(|m| m.as_str()).unwrap_or("").to_string(),
                },
                Some("fatal") => Reply::Fatal {
                    hresult: v.get("hresult").and_then(|h| h.as_i64()).unwrap_or(0) as i32,
                    msg: v.get("msg").and_then(|m| m.as_str()).unwrap_or("").to_string(),
                },
                _ => continue,
            };
            if tx.send(reply).is_err() {
                break;
            }
        }
    });
    rx
}

/// 按应用缓存 worker，整批复用。
pub struct OfficePool {
    script: PathBuf,
    workers: HashMap<OfficeApp, OfficeWorker>,
    /// 每个应用最近一次启动失败的原因，避免反复付出超时代价
    failures: HashMap<OfficeApp, String>,
}

impl OfficePool {
    pub fn new(script: PathBuf) -> Self {
        OfficePool { script, workers: HashMap::new(), failures: HashMap::new() }
    }

    /// 取一个可用的 worker，必要时惰性启动。
    pub fn worker(&mut self, app: OfficeApp) -> Result<&mut OfficeWorker, String> {
        if let Some(reason) = self.failures.get(&app) {
            return Err(reason.clone());
        }
        if !self.workers.contains_key(&app) {
            match OfficeWorker::start(app, self.script.clone()) {
                Ok(w) => {
                    self.workers.insert(app, w);
                }
                Err(e) => {
                    self.failures.insert(app, e.clone());
                    return Err(e);
                }
            }
        }
        Ok(self.workers.get_mut(&app).expect("刚插入过"))
    }

    /// 转换失败后丢弃该 worker，下次调用会重建。
    /// Word 崩溃或超时后这是唯一能恢复的手段。
    pub fn discard(&mut self, app: OfficeApp) {
        if let Some(mut w) = self.workers.remove(&app) {
            w.kill();
        }
    }

    /// 记录一次瞬时故障，让后续文件仍会尝试重建 worker，
    /// 但把「这台机器根本没装」这类硬失败缓存住。
    pub fn note_hard_failure(&mut self, app: OfficeApp, reason: String) {
        self.failures.insert(app, reason);
    }

    pub fn clear_failure(&mut self, app: OfficeApp) {
        self.failures.remove(&app);
    }

    pub fn shutdown_all(&mut self) {
        for (_, mut w) in self.workers.drain() {
            w.shutdown();
        }
    }
}

impl Drop for OfficePool {
    fn drop(&mut self) {
        self.shutdown_all();
    }
}
