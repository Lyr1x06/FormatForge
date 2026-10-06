import { memo } from 'react';
import { IconAlert, IconArrowRight, IconCheck, IconClose, IconDownload, IconLock } from './icons.jsx';
import { glyph, tint } from '../lib/format-meta.js';
import { bytes, sizeDelta } from '../lib/format.js';
import ProgressRing from './ProgressRing.jsx';

const STATE_LABEL = {
  queued: '排队中',
  preparing: '准备中',
  converting: '转换中',
  done: '完成',
  failed: '失败',
  cancelled: '已取消',
};

function FileRow({ file, index, status, target, onRemove, onReveal }) {
  const state = status?.state ?? 'idle';
  const p = status?.p ?? 0;
  const error = status?.error;
  const showProgress = state === 'converting' || state === 'preparing';

  const delta =
    state === 'done' && status?.bytesIn && status?.bytesOut
      ? sizeDelta(status.bytesIn, status.bytesOut)
      : null;

  const blocked = file.unsupported || (target && !file.targets.includes(target));

  return (
    <div
      className={`filerow is-${state}${blocked ? ' is-blocked' : ''}`}
      style={{ '--i': Math.min(index, 12) }}
    >
      <span className="thumb" style={{ background: `${tint(file.format)}22` }}>
        <span className="ext" style={{ color: tint(file.format) }}>
          {glyph(file.format)}
        </span>
      </span>

      <div className="row-main">
        <div className="row-name" title={file.path}>
          {file.name}
        </div>
        <div className="row-meta">
          {file.rel && (
            <>
              <span className="relpath" title={file.rel}>
                {file.rel}
              </span>
              <span className="sep">·</span>
            </>
          )}
          <span>{bytes(file.bytes)}</span>
          {file.mismatch && (
            <>
              <span className="sep">·</span>
              <span className="warn-chip" title={`扩展名与内容不符，实际是 ${file.format_label}`}>
                <IconAlert size={10} />
                实际是 {file.format_label}
              </span>
            </>
          )}
          {blocked && !file.unsupported && (
            <>
              <span className="sep">·</span>
              <span className="warn-chip">无法转为该格式</span>
            </>
          )}
          {file.unsupported && (
            <>
              <span className="sep">·</span>
              <span className="warn-chip">不支持</span>
            </>
          )}
        </div>
      </div>

      <div className="flow">
        <span className="src">{file.format}</span>
        <span className="arrow">
          <IconArrowRight size={11} />
        </span>
        <span className={`dst${blocked ? ' none' : ''}`}>{target || '—'}</span>
      </div>

      <div className="row-status">
        {delta && (
          <span className={`delta ${delta.smaller ? 'smaller' : 'bigger'}`}>
            {delta.smaller ? '' : '+'}
            {delta.text.replace('减少 ', '-').replace('增加 ', '+').replace(/\s.*$/, '')}
          </span>
        )}
        <span className={`status-chip ${state}`}>
          {state === 'done' ? <IconCheck size={10} /> : null}
          {state === 'password' ? <IconLock size={10} /> : null}
          {STATE_LABEL[state] ?? ''}
        </span>
      </div>

      <div className="row-actions">
        {showProgress && <ProgressRing p={p} done={false} />}
        {state === 'done' && status?.outPath && (
          <button
            className="icon-btn"
            title="在文件夹中显示"
            onClick={() => onReveal?.(status.outPath)}
          >
            <IconDownload size={14} />
          </button>
        )}
        <button
          className="icon-btn"
          title="移除"
          onClick={() => onRemove(file.id)}
          disabled={!onRemove}
        >
          <IconClose size={14} />
        </button>
      </div>

      {error && <div className="row-error">{error.message}</div>}
    </div>
  );
}

export default memo(FileRow);
