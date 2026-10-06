//! 前端可调用的命令。
//!
//! 前端只通过这些命令与后端交互，不直接触碰任何转换逻辑。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{Manager, State};

use crate::engine::{OfficePool, ProbeResult};
use crate::job::{BatchRun, CancelToken, JobEvent, Reporter};
use crate::registry::{self, FormatDef};
use crate::scan::{self, ScanRequest, ScanResult};

/// 应用的全局状态
pub struct AppState {
    pub office_pool: Arc<Mutex<OfficePool>>,
    /// 进行中的批量：批次号 → 取消令牌
    pub running: Mutex<HashMap<u32, Arc<CancelToken>>>,
    pub next_batch: AtomicU32,
}

impl AppState {
    pub fn new(script: PathBuf) -> Self {
        AppState {
            office_pool: Arc::new(Mutex::new(OfficePool::new(script))),
            running: Mutex::new(HashMap::new()),
            next_batch: AtomicU32::new(1),
        }
    }
}

/// office-worker.ps1 的位置。开发期在 src-tauri/scripts 下，
/// 打包后随资源一起分发。
fn worker_script(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    // 打包后优先用随包资源
    if let Ok(res) = app.path().resource_dir() {
        let p = res.join("scripts").join("office-worker.ps1");
        if p.is_file() {
            return Ok(p);
        }
    }
    // 开发期：相对 CARGO_MANIFEST_DIR
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("office-worker.ps1");
    if dev.is_file() {
        return Ok(dev);
    }
    Err("找不到 office-worker.ps1".into())
}

/// setup 钩子里用；失败时退化成「没有 Office 引擎」而不是让应用起不来。
pub fn worker_script_for_setup(app: &tauri::AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(worker_script(app).unwrap_or_default())
}

/* ---------------------------------------------------------------- 引擎 */

#[tauri::command]
pub fn probe_engines() -> ProbeResult {
    crate::engine::probe::probe_all()
}

/// 用系统默认浏览器打开一个链接（LibreOffice 安装引导用）
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    crate::engine::probe::open_external(url)
}

/* ---------------------------------------------------------------- 格式表 */

#[tauri::command]
pub fn list_formats() -> Vec<FormatDef> {
    registry::registry()
}

/* ---------------------------------------------------------------- 扫描 */

#[derive(Debug, Deserialize)]
pub struct StartRequest {
    pub files: Vec<scan::ScannedFile>,
    pub dst_format: String,
    pub output_dir: String,
    /// 参数面板的值。目前仅在 PDF/Office 引擎选择与图片质量上生效；
    /// 未消费的键留着不报错，避免前端加一个控件就要改后端。
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

#[tauri::command]
pub async fn scan_inputs(req: ScanRequest) -> Result<ScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || scan::scan(&req))
        .await
        .map_err(|e| format!("扫描失败：{e}"))
}

/* ---------------------------------------------------------------- 转换 */

#[derive(Debug, Clone, Serialize)]
pub struct StartAck {
    pub batch_id: u32,
    pub planned: usize,
    pub notices: Vec<String>,
}

#[tauri::command]
pub async fn start_batch(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    on_event: Channel<JobEvent>,
    req: StartRequest,
) -> Result<StartAck, String> {
    let output_dir = PathBuf::from(&req.output_dir);
    if req.output_dir.trim().is_empty() {
        return Err("请先选择输出目录".into());
    }
    if !output_dir.is_dir() {
        // 输出目录不存在就建出来，比报错友好
        std::fs::create_dir_all(crate::paths::for_io(&output_dir))
            .map_err(|e| format!("无法创建输出目录：{e}"))?;
    }

    let (specs, notices) = scan::plan(
        &req.files,
        &req.dst_format,
        &output_dir,
        &req.options,
        |f| {
            PathBuf::from(&f.name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| f.name.clone())
        },
    );

    if specs.is_empty() {
        return Err("没有可转换的文件——请检查目标格式与所选文件是否匹配".into());
    }

    let batch_id = state.next_batch.fetch_add(1, Ordering::SeqCst);
    let cancel = Arc::new(CancelToken::default());
    if let Ok(mut map) = state.running.lock() {
        map.insert(batch_id, Arc::clone(&cancel));
    }

    let total = specs.len();
    let reporter = Arc::new(Reporter::new(on_event, total));
    let pool = Arc::clone(&state.office_pool);

    let batch = BatchRun {
        specs,
        output_dir,
        cancel,
        reporter,
        office_pool: pool,
        notices: notices.clone(),
    };

    let handle = app.clone();

    // 转换是同步阻塞的重活，丢到专用线程，别占住 async 运行时
    std::thread::spawn(move || {
        crate::job::run(batch);
        if let Some(st) = handle.try_state::<AppState>() {
            if let Ok(mut map) = st.running.lock() {
                map.remove(&batch_id);
            }
            // 整批结束后收掉 worker，避免 Office 进程常驻占内存
            if let Ok(mut pool) = st.office_pool.lock() {
                pool.shutdown_all();
            }
        }
    });

    Ok(StartAck { batch_id, planned: total, notices })
}

#[tauri::command]
pub fn cancel_batch(state: State<'_, AppState>, batch_id: u32) -> Result<(), String> {
    let map = state.running.lock().map_err(|_| "状态异常")?;
    match map.get(&batch_id) {
        Some(token) => {
            token.cancel();
            Ok(())
        }
        None => Err("该批次已结束".into()),
    }
}

/// 重试失败的单个文件：清掉该应用的硬失败缓存，让下次调用重新尝试启动。
#[tauri::command]
pub fn clear_engine_failure(
    state: State<'_, AppState>,
    src_format: String,
) -> Result<(), String> {
    let Some(app) = crate::engine::OfficeApp::from_ext(&src_format) else {
        return Ok(());
    };
    if let Ok(mut pool) = state.office_pool.lock() {
        pool.clear_failure(app);
    }
    Ok(())
}
