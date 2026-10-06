//! 批量作业模型与调度。
//!
//! 两条泳道并行：
//!   * **本地泳道**——图片编解码、纯 Rust 的文档解析。CPU 密集，按核数并发。
//!   * **Office 泳道**——COM 转换。有状态的单实例，必须严格串行。
//!
//! 分开的理由：混装批量里图片和文档能同时推进，而不是互相排队。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::engine::{FailureKind, OfficeApp, OfficePool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Preparing,
    Converting,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, JobState::Done | JobState::Failed | JobState::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobError {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobSpec {
    pub id: u32,
    pub src: String,
    /// 嗅探到的真实格式 id
    pub src_format: String,
    pub dst_format: String,
    pub dst: String,
    /// 相对投放文件夹的路径，仅用于展示
    pub rel: Option<String>,
    /// 参数面板的值，沿用用户选的
    #[serde(skip)]
    pub options: std::collections::HashMap<String, serde_json::Value>,
}

impl JobSpec {
    /// 取一个数值参数，缺失或类型不对时用兜底值
    pub fn num(&self, key: &str, fallback: f64) -> f64 {
        self.options.get(key).and_then(|v| v.as_f64()).unwrap_or(fallback)
    }

    pub fn text<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.options.get(key).and_then(|v| v.as_str()).unwrap_or(fallback)
    }

    pub fn flag(&self, key: &str, fallback: bool) -> bool {
        self.options.get(key).and_then(|v| v.as_bool()).unwrap_or(fallback)
    }
}

/// 事件流。
///
/// 注意 `rename_all` 作用于变体名（→ camelCase 的 kind 值），
/// 字段名要靠 `rename_all_fields` 才会一起变成 camelCase。
/// 少了后者，前端会收到 `bytes_out` 而它期待的是 `bytesOut`——静默失效。
///
/// `Deserialize` 是给测试用的：测试里把 Channel 收到的 JSON 还原成事件做断言。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum JobEvent {
    /// 整批开始
    Started { total: usize, output_dir: String },
    /// 状态跃迁——必发，前端据此改变行外观
    State {
        id: u32,
        state: JobState,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<JobError>,
    },
    /// 文件内进度——已节流
    Progress { id: u32, p: f32 },
    /// 单个文件产出
    Output {
        id: u32,
        /// 实际落地的输出路径，供「在文件夹中显示」使用
        dst: String,
        bytes_in: u64,
        bytes_out: u64,
        ms: u64,
    },
    /// 聚合进度——约 10Hz
    Tick {
        done: usize,
        total: usize,
        failed: usize,
        bytes_out: u64,
        elapsed_ms: u64,
        eta_ms: Option<u64>,
    },
    /// 整批结束
    Finished {
        ok: bool,
        cancelled: bool,
        succeeded: usize,
        failed: usize,
        bytes_out: u64,
        ms: u64,
    },
    /// 需要展示给用户的信息（引擎降级、超长路径等）
    Notice { level: String, message: String },
}

/// 取消令牌。作业间隙检查；COM 调用飞行中则靠杀 worker 实现。
#[derive(Default)]
pub struct CancelToken {
    cancelled: AtomicBool,
}

impl CancelToken {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// 进度上报器。负责事件节流与聚合，避免高频刷新打爆 WebView。
pub struct Reporter {
    tx: tauri::ipc::Channel<JobEvent>,
    total: usize,
    done: AtomicUsize,
    failed: AtomicUsize,
    bytes_out: AtomicUsize,
    started: Instant,
    last_tick: Mutex<Instant>,
    /// 每个作业上一次上报的进度，用于抑制无意义的重复事件
    last_p: Mutex<Vec<f32>>,
}

impl Reporter {
    pub fn new(tx: tauri::ipc::Channel<JobEvent>, total: usize) -> Self {
        Reporter {
            tx,
            total,
            done: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            bytes_out: AtomicUsize::new(0),
            started: Instant::now(),
            last_tick: Mutex::new(Instant::now() - Duration::from_secs(1)),
            last_p: Mutex::new(vec![0.0; total]),
        }
    }

    pub fn state(&self, id: u32, state: JobState, error: Option<JobError>) {
        let _ = self.tx.send(JobEvent::State { id, state, error });
        if state.is_terminal() {
            self.bump(state);
        }
    }

    /// 进度上报，节流到约 10Hz 或 1% 变化。
    pub fn progress(&self, id: u32, p: f32) {
        let p = p.clamp(0.0, 1.0);
        {
            let mut guard = match self.last_p.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let prev = guard.get(id as usize).copied().unwrap_or(0.0);
            // 1% 以内且不是首尾，直接丢弃
            if (p - prev).abs() < 0.01 && p > 0.0 && p < 1.0 {
                return;
            }
            if id as usize >= guard.len() {
                return;
            }
            guard[id as usize] = p;
        }
        let _ = self.tx.send(JobEvent::Progress { id, p });
    }

    pub fn output(&self, id: u32, dst: &str, bytes_in: u64, bytes_out: u64, ms: u64) {
        self.bytes_out.fetch_add(bytes_out as usize, Ordering::Relaxed);
        let _ = self.tx.send(JobEvent::Output {
            id,
            dst: dst.to_string(),
            bytes_in,
            bytes_out,
            ms,
        });
    }

    pub fn notice(&self, level: &str, message: impl Into<String>) {
        let _ = self.tx.send(JobEvent::Notice { level: level.into(), message: message.into() });
    }

    /// 直发一条事件，不参与节流。给批次级的 Started 用。
    pub fn send_raw(&self, ev: JobEvent) {
        let _ = self.tx.send(ev);
    }

    fn bump(&self, state: JobState) {
        match state {
            JobState::Done => {
                self.done.fetch_add(1, Ordering::Relaxed);
            }
            JobState::Failed => {
                self.failed.fetch_add(1, Ordering::Relaxed);
            }
            _ => return,
        }
        self.maybe_tick(false);
    }

    /// 聚合进度。强制发（收尾）时忽略节流。
    pub fn maybe_tick(&self, force: bool) {
        {
            let mut last = match self.last_tick.lock() {
                Ok(l) => l,
                Err(_) => return,
            };
            if !force && last.elapsed() < Duration::from_millis(100) {
                return;
            }
            *last = Instant::now();
        }

        let done = self.done.load(Ordering::Relaxed);
        let failed = self.failed.load(Ordering::Relaxed);
        let finished = done + failed;
        let elapsed = self.started.elapsed();

        // 用「已完成 / 总数」估剩余时间；前几个文件的均值噪声大，所以
        // 至少要完成 2 个才给 ETA
        let eta_ms = if finished >= 2 && finished < self.total {
            let per = elapsed.as_millis() as f64 / finished as f64;
            Some((per * (self.total - finished) as f64) as u64)
        } else {
            None
        };

        let _ = self.tx.send(JobEvent::Tick {
            done: finished,
            total: self.total,
            failed,
            bytes_out: self.bytes_out.load(Ordering::Relaxed) as u64,
            elapsed_ms: elapsed.as_millis() as u64,
            eta_ms,
        });
    }

    pub fn finish(&self, cancelled: bool) {
        self.maybe_tick(true);
        let succeeded = self.done.load(Ordering::Relaxed);
        let failed = self.failed.load(Ordering::Relaxed);
        let _ = self.tx.send(JobEvent::Finished {
            ok: failed == 0,
            cancelled,
            succeeded,
            failed,
            bytes_out: self.bytes_out.load(Ordering::Relaxed) as u64,
            ms: self.started.elapsed().as_millis() as u64,
        });
    }
}

/// 一次批量运行的输入
pub struct BatchRun {
    pub specs: Vec<JobSpec>,
    pub output_dir: PathBuf,
    pub cancel: Arc<CancelToken>,
    pub reporter: Arc<Reporter>,
    pub office_pool: Arc<Mutex<OfficePool>>,
    /// 规划阶段发现的问题（无法转换的组合、超长路径等），开跑前先告诉用户
    pub notices: Vec<String>,
}

/// 把作业按泳道分组
fn partition(specs: &[JobSpec]) -> (Vec<usize>, Vec<usize>) {
    let mut office = Vec::new();
    let mut local = Vec::new();
    for (i, s) in specs.iter().enumerate() {
        if s.dst_format == "pdf" && OfficeApp::from_ext(&s.src_format).is_some() {
            office.push(i);
        } else {
            local.push(i);
        }
    }
    (office, local)
}

/// 执行一整批。阻塞直到完成或被取消。
pub fn run(batch: BatchRun) {
    let BatchRun { specs, output_dir, cancel, reporter, office_pool, notices } = batch;

    let _ = reporter.send_raw(JobEvent::Started {
        total: specs.len(),
        output_dir: output_dir.to_string_lossy().to_string(),
    });

    // 规划阶段的提示先发出去，用户能在行状态变化前就看到
    for n in notices {
        reporter.notice("info", n);
    }

    let specs = Arc::new(specs);
    let (office_idx, local_idx) = partition(&specs);

    let mut handles = Vec::new();

    // 本地泳道：按核数并发
    if !local_idx.is_empty() {
        let queue = Arc::new(Mutex::new(local_idx));

        for _ in 0..num_cpus::get().clamp(1, 8) {
            let specs = Arc::clone(&specs);
            let out = output_dir.clone();
            let cancel = Arc::clone(&cancel);
            let reporter = Arc::clone(&reporter);
            let queue = Arc::clone(&queue);

            handles.push(std::thread::spawn(move || loop {
                if cancel.is_cancelled() {
                    // 剩余排队的标记为已取消，前端要如实反映
                    let rest: Vec<u32> = match queue.lock() {
                        Ok(mut q) => q.drain(..).map(|i| specs[i].id).collect(),
                        Err(_) => return,
                    };
                    for id in rest {
                        reporter.state(id, JobState::Cancelled, None);
                    }
                    return;
                }
                let idx = match queue.lock() {
                    Ok(mut q) => q.pop(),
                    Err(_) => return,
                };
                let Some(idx) = idx else { return };
                crate::convert::run_local(&specs[idx], &out, &cancel, &reporter);
            }));
        }
    }

    // Office 泳道：严格串行。COM 实例是有状态的单例。
    if !office_idx.is_empty() {
        let specs = Arc::clone(&specs);
        let out = output_dir.clone();
        let cancel = Arc::clone(&cancel);
        let reporter = Arc::clone(&reporter);

        handles.push(std::thread::spawn(move || {
            for idx in office_idx {
                if cancel.is_cancelled() {
                    reporter.state(specs[idx].id, JobState::Cancelled, None);
                    continue;
                }
                crate::convert::run_office_pdf(
                    &specs[idx],
                    &out,
                    &cancel,
                    &reporter,
                    &office_pool,
                );
            }
        }));
    }

    for h in handles {
        let _ = h.join();
    }

    reporter.finish(cancel.is_cancelled());
}

/// 把失败分类转成前端可展示的错误
pub fn job_error(kind: FailureKind, app: OfficeApp, detail: String) -> JobError {
    let mut message = kind.user_message(app);
    // 原始消息留在后面，方便排查；但对用户来说主信息是上面那句
    let detail = detail.trim();
    if !detail.is_empty() && kind == FailureKind::Unknown {
        message = format!("{message}：{detail}");
    }
    JobError { kind: format!("{kind:?}").to_lowercase(), message }
}

pub fn simple_error(kind: &str, message: impl Into<String>) -> JobError {
    JobError { kind: kind.into(), message: message.into() }
}
