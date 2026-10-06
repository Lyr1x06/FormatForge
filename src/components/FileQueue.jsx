import FileRow from './FileRow.jsx';

/**
 * 文件队列。
 *
 * 每行自己的状态切片通过 props 传下去，FileRow 上了 memo，
 * 所以一个文件的进度更新只会重渲染那一行，不会波及整棵树。
 */
export default function FileQueue({ files, status, target, collapsing, onRemove, onReveal }) {
  return (
    <div className="filelist">
      {files.map((f, i) => (
        <div
          key={f.id}
          className={`row-wrap${collapsing.has(f.id) ? ' collapsing' : ''}`}
        >
          <div className="row-clip">
            <FileRow
              file={f}
              index={i}
              status={status[f.id]}
              target={target}
              onRemove={onRemove}
              onReveal={onReveal}
            />
          </div>
        </div>
      ))}
    </div>
  );
}
