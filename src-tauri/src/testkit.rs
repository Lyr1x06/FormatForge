//! 测试支撑。
//!
//! 端到端测试需要绕过 Tauri 的运行时（拿不到 AppHandle，也就建不出
//! `Channel`）直接驱动作业管线。这个模块把那条路铺出来，同时把
//! `Channel` 换成一个收集事件的替身。
//!
//! 不是给生产代码用的，放在 lib 里只是为了让 `tests/` 能引用到。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::engine::OfficePool;
use crate::job::{BatchRun, CancelToken, JobEvent, JobSpec, Reporter};
use crate::scan::{self, ScanRequest, ScannedFile};

/// 一次批量跑完之后的全部事件与统计
#[derive(Debug)]
pub struct Outcome {
    pub events: Vec<JobEvent>,
    /// 规划出来的输出路径。测试要断言「产物落在哪」时用它，
    /// 不要靠猜文件名——命名阶梯会因为冲突而加 ` (2)` 后缀。
    pub planned_outputs: Vec<PathBuf>,
}

impl Outcome {
    pub fn succeeded(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, JobEvent::State { state: crate::job::JobState::Done, .. }))
            .count()
    }

    pub fn failed(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, JobEvent::State { state: crate::job::JobState::Failed, .. }))
            .count()
    }

    /// 第一个计划输出路径。单文件批次里就是那个产物。
    pub fn output(&self) -> &Path {
        self.planned_outputs
            .first()
            .expect("本批没有规划出任何输出")
    }

    /// 某一次失败的错误信息，测试里拿来断言分类对不对
    pub fn first_error(&self) -> Option<&str> {
        self.events.iter().find_map(|e| match e {
            JobEvent::State { error: Some(err), .. } => Some(err.message.as_str()),
            _ => None,
        })
    }
}

/// 收集事件的 Channel 替身。
///
/// `Reporter` 里那个 `Channel<JobEvent>` 只用来 `send`，所以这里包一层：
/// 真正的 `Channel` 需要 Tauri 运行时，测试里用一个函数式替身即可。
fn event_channel(sink: Arc<Mutex<Vec<JobEvent>>>) -> tauri::ipc::Channel<JobEvent> {
    tauri::ipc::Channel::new(move |body| {
        let json = match body {
            tauri::ipc::InvokeResponseBody::Json(s) => s,
            // 事件走 Json 分支；别的形态不该出现，出现了就当协议错
            _ => return Ok(()),
        };
        if let Ok(ev) = serde_json::from_str::<JobEvent>(&json) {
            if let Ok(mut v) = sink.lock() {
                v.push(ev);
            }
        }
        Ok(())
    })
}

/// 建一个干净的临时目录。同名目录已存在时先清空，避免测试间互相污染。
pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("format-forge-test-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时目录");
    dir
}

pub fn write_text(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, content).expect("写测试文件");
}

/// 只跑扫描与规划，不转换。用来验证嗅探、目标交集、命名去重。
pub fn scan_and_plan(
    paths: &[String],
    dst_format: &str,
    out_dir: &Path,
) -> (Vec<ScannedFile>, Vec<JobSpec>) {
    let result = scan::scan(&ScanRequest {
        paths: paths.to_vec(),
        recurse: true,
        max_depth: 8,
    });

    let (specs, _notices) = scan::plan(
        &result.files,
        dst_format,
        out_dir,
        &std::collections::HashMap::new(),
        |f| {
            Path::new(&f.name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| f.name.clone())
        },
    );

    (result.files, specs)
}

/// 跑一整批，阻塞到结束。Office 泳道不出网——只跑本地泳道的转换。
pub fn run_batch_sync(paths: &[String], dst_format: &str, out_dir: &Path) -> Outcome {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let channel = event_channel(Arc::clone(&sink));

    let (_, specs) = scan_and_plan(paths, dst_format, out_dir);
    let outputs: Vec<PathBuf> = specs.iter().map(|s| PathBuf::from(&s.dst)).collect();

    // Office worker 脚本路径给一个不存在的——测试里不碰 COM，
    // 真碰上 Office 泳道的作业会得到一个明确的「启动失败」而不是挂住。
    let pool = Arc::new(Mutex::new(OfficePool::new(PathBuf::from(
        "nonexistent-office-worker.ps1",
    ))));

    let reporter = Arc::new(Reporter::new(channel, specs.len().max(1)));

    crate::job::run(BatchRun {
        specs,
        output_dir: out_dir.to_path_buf(),
        cancel: Arc::new(CancelToken::default()),
        reporter,
        office_pool: pool,
        notices: Vec::new(),
    });

    let events = sink.lock().map(|v| v.clone()).unwrap_or_default();
    Outcome { events, planned_outputs: outputs }
}


/// 用指定的参数跑一批已经规划好的作业。
///
/// 规划与执行分开是为了验证「动作注入的参数」——`@split` 在扫描阶段被翻译成
/// `pdf_op=split`，测试里想换其中若干项就得绕开 `plan` 自己拼一遍。
pub fn run_specs_with_options(
    mut specs: Vec<JobSpec>,
    out_dir: &Path,
    overrides: &[(&str, &str)],
) -> Outcome {
    for spec in specs.iter_mut() {
        for (k, v) in overrides {
            spec.options.insert(
                (*k).to_string(),
                serde_json::Value::String((*v).to_string()),
            );
        }
    }
    run_specs(specs, out_dir)
}

/// 直接跑一批作业，绕过扫描与规划。
pub fn run_specs(specs: Vec<JobSpec>, out_dir: &Path) -> Outcome {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let channel = event_channel(Arc::clone(&sink));
    let total = specs.len().max(1);
    let outputs: Vec<PathBuf> = specs.iter().map(|s| PathBuf::from(&s.dst)).collect();

    let pool = Arc::new(Mutex::new(OfficePool::new(PathBuf::from(
        "nonexistent-office-worker.ps1",
    ))));
    let reporter = Arc::new(Reporter::new(channel, total));

    crate::job::run(BatchRun {
        specs,
        output_dir: out_dir.to_path_buf(),
        cancel: Arc::new(CancelToken::default()),
        reporter,
        office_pool: pool,
        notices: Vec::new(),
    });

    Outcome {
        events: sink.lock().map(|v| v.clone()).unwrap_or_default(),
        planned_outputs: outputs,
    }
}

/// 注册表序列化后的 JSON。测试用它校验跨语言的形状约定。
pub fn list_formats_json() -> serde_json::Value {
    serde_json::to_value(crate::registry::registry()).expect("注册表必须能序列化")
}
