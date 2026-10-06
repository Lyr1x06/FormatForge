import { IconFolder, IconLayers, IconPlus } from './icons.jsx';
import { PrivacyNote } from './TopBar.jsx';

export default function DropZone({
  onPickFiles,
  onPickFolder,
  scanning,
  error,
  formats,
  formatsLoading,
}) {
  const summary = formatsLoading ? null : summarize(formats);

  return (
    <div className="dropzone">
      <div className="dz-icon">{scanning ? <IconLayers size={26} /> : <IconPlus size={26} />}</div>

      <h2>{scanning ? '正在读取文件…' : '把文件拖到这里'}</h2>

      <p className="hint">
        {error
          ? error
          : summary
            ? `支持 ${summary}`
            : '支持图片与文档/数据格式，可拖入整个文件夹'}
      </p>

      {!scanning && (
        <div className="dz-actions">
          <button className="btn-ghost" onClick={onPickFiles}>
            <IconPlus size={15} />
            选择文件
          </button>
          <button className="btn-ghost" onClick={onPickFolder}>
            <IconFolder size={15} />
            选择文件夹
          </button>
        </div>
      )}

      <PrivacyNote />
    </div>
  );
}

/** 从注册表里算出「支持哪些格式」，避免在界面里手写一份会过期的清单 */
function summarize(formats) {
  const list = Object.values(formats).filter((f) => f.can_decode);
  const images = list.filter((f) => f.category === 'image').length;
  const docs = list.filter((f) => f.category !== 'image').length;
  return `${images} 种图片 · ${docs} 种文档与数据格式`;
}
