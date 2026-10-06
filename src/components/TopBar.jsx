import { IconFolder, IconShield, LogoMark } from './icons.jsx';
import { ellipsisPath } from '../lib/format.js';

const FIDELITY_LABEL = {
  exact: '1:1 保真',
  near: '近乎 1:1',
  relayout: '会重新排版',
};

export default function TopBar({
  engine,
  probing,
  outputDir,
  onPickOutput,
  onShowEngines,
  running,
}) {
  return (
    <header className="topbar">
      <div className="brand">
        <div className="brand-mark">
          <LogoMark size={19} />
        </div>
        <div>
          <h1>Format Forge</h1>
          <div className="sub">本地批量格式转换</div>
        </div>
      </div>

      <div className="topbar-right">
        <button
          className="outdir"
          onClick={onPickOutput}
          disabled={running}
          title={outputDir || '点击选择输出目录'}
        >
          <IconFolder size={14} />
          <span className={`path${outputDir ? '' : ' empty'}`}>
            {outputDir ? ellipsisPath(outputDir, 40) : '选择输出目录'}
          </span>
        </button>

        <button className="engine-chip" onClick={onShowEngines} title="查看转换引擎详情">
          <span className={`dot ${dotClass(engine, probing)}`} />
          <span className="label">
            {probing ? '检测中…' : (engine?.label ?? '无可用引擎')}
          </span>
          {engine && <span className="note">{FIDELITY_LABEL[engine.fidelity]}</span>}
          {!engine && !probing && <span className="note">文档转 PDF 不可用</span>}
        </button>
      </div>
    </header>
  );
}

function dotClass(engine, probing) {
  if (probing) return 'none';
  return engine?.fidelity ?? 'none';
}

/** 页脚那句隐私声明。放在这里是因为它属于「产品属性」而不是某个界面。 */
export function PrivacyNote() {
  return (
    <span className="privacy">
      <IconShield size={13} />
      全部本地处理，文件不出本机
    </span>
  );
}
