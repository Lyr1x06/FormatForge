import { IconCheck, IconFolderOpen, IconRetry, IconSparkle, IconStop } from './icons.jsx';
import { bytes, duration, eta } from '../lib/format.js';

/**
 * 底部动作条。四个阶段在同一个位置就地变形：
 * 待命 → 转换中 → 结果摘要 → 回到待命。
 */
export default function ConvertBar({
  phase,
  tick,
  summary,
  canRun,
  target,
  fileCount,
  onRun,
  onCancel,
  onOpenOutput,
  onReset,
}) {
  if (phase === 'running') {
    return <RunningBar tick={tick} onCancel={onCancel} />;
  }

  if (phase === 'done' && summary) {
    return (
      <SummaryBar
        summary={summary}
        onOpenOutput={onOpenOutput}
        onReset={onReset}
        onRerun={onRun}
      />
    );
  }

  return (
    <IdleBar
      canRun={canRun}
      target={target}
      fileCount={fileCount}
      onRun={onRun}
    />
  );
}

/* ---------------------------------------------------------------- 待命 */

function IdleBar({ canRun, target, fileCount, onRun }) {
  return (
    <div className="actionbar">
      <div className="bar-inner">
        <div className="bar-info">
          <div className="bar-line">
            <span className="strong">{fileCount}</span>
            <span>个文件待转换</span>
            {target && (
              <>
                <span className="sep">·</span>
                <span>
                  目标 <span className="strong">{target}</span>
                </span>
              </>
            )}
          </div>
          <div className="bar-track">
            <div className="bar-fill idle" />
          </div>
        </div>
        <button className="btn-primary" onClick={onRun} disabled={!canRun}>
          <IconSparkle size={15} />
          开始转换
        </button>
      </div>
    </div>
  );
}

/* ---------------------------------------------------------------- 进行中 */

function RunningBar({ tick, onCancel }) {
  const done = tick?.done ?? 0;
  const total = tick?.total ?? 0;
  const p = total > 0 ? done / total : 0;
  const remaining = eta(tick?.etaMs);

  return (
    <div className="actionbar">
      <div className="bar-inner">
        <div className="bar-info">
          <div className="bar-line">
            <span className="strong">
              {done} / {total}
            </span>
            <span>{tick?.failed > 0 ? `${tick.failed} 个失败` : '转换中'}</span>
            <span className="eta">
              {duration(tick?.elapsedMs ?? 0)}
              {remaining ? ` · 剩余${remaining.replace('约 ', ' ')}` : ''}
            </span>
          </div>
          <div className="bar-track">
            <div
              className={`bar-fill${tick?.failed > 0 ? ' failed' : ''}`}
              style={{ '--p': p }}
            />
          </div>
        </div>
        <button className="btn-primary cancel" onClick={onCancel}>
          <IconStop size={14} />
          取消
        </button>
      </div>
    </div>
  );
}

/* ---------------------------------------------------------------- 结果 */

function SummaryBar({ summary, onOpenOutput, onReset, onRerun }) {
  const { succeeded, failed, cancelled, bytesOut, ms } = summary;
  const good = failed === 0 && !cancelled;

  return (
    <div className="actionbar">
      <div className="bar-inner">
        <div className="bar-info">
          <div className="bar-line">
            <span className={`summary-icon${good ? ' good' : ' bad'}`}>
              {good ? <IconCheck size={13} /> : <IconStop size={13} />}
            </span>
            <span className="strong">
              {cancelled ? '已取消' : good ? '全部完成' : `${failed} 个失败`}
            </span>
            <span className="eta">{duration(ms)}</span>
          </div>

          <div className="result-stats">
            <span className="stat">
              <span className="num good">{succeeded}</span> 成功
            </span>
            {failed > 0 && (
              <span className="stat">
                <span className="num bad">{failed}</span> 失败
              </span>
            )}
            <span className="stat">
              <span className="num">{bytes(bytesOut)}</span> 产出
            </span>
          </div>
        </div>

        {failed > 0 && (
          <button className="btn-ghost" onClick={onRerun}>
            <IconRetry size={14} />
            重试失败项
          </button>
        )}
        <button className="btn-ghost" onClick={onOpenOutput}>
          <IconFolderOpen size={15} />
          打开输出目录
        </button>
        <button className="btn-primary" onClick={onReset}>
          再来一批
        </button>
      </div>
    </div>
  );
}

/* ---------------------------------------------------------------- 结果 */
