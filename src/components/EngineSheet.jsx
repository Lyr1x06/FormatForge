import { useEffect, useState } from 'react';
import { IconClose, IconRefresh, IconShield } from './icons.jsx';
import { openUrl } from '../lib/api.js';

const FIDELITY = {
  exact: { label: '1:1 保真', cls: 'exact' },
  near: { label: '近乎 1:1', cls: 'near' },
  relayout: { label: '会重新排版', cls: 'relayout' },
};

/**
 * 引擎详情与安装引导。
 *
 * 「保真度」是这台机器上真实可用的引擎能给出的结果，所以必须如实标注。
 * 把 LibreOffice 说成「和 Office 一样」是最容易犯也最伤人的错误。
 */
export default function EngineSheet({ report, probing, onRefresh, onClose }) {
  useEffect(() => {
    const onKey = (e) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  const engines = report?.engines ?? [];
  const wingetId = report?.libreoffice_winget_id ?? 'TheDocumentFoundation.LibreOffice';
  const [copied, setCopied] = useState(false);

  const copyCmd = async () => {
    try {
      await navigator.clipboard.writeText(
        `winget install --id ${wingetId} -e --accept-package-agreements --accept-source-agreements`,
      );
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      // 剪贴板不可用时不打扰用户，命令在界面上本来就是可选的
    }
  };

  return (
    <>
      <div className="veil" onClick={onClose} />
      <div className="sheet" role="dialog" aria-modal="true" aria-label="转换引擎" style={{ width: 'min(620px, 100%)' }}>
        <div className="sheet-head">
          <h2>转换引擎</h2>
          <div style={{ display: 'flex', gap: 6 }}>
            <button className="icon-btn" onClick={onRefresh} title="重新检测">
              <IconRefresh size={15} />
            </button>
            <button className="sheet-close" onClick={onClose} aria-label="关闭">
              <IconClose size={16} />
            </button>
          </div>
        </div>

        <p className="sheet-lead">
          Office 文档转 PDF 会按保真度从高到低选择引擎。
          本机当前探测结果如下。
        </p>

        <div className="engine-list">
          {probing && engines.length === 0 && (
            <>
              <div className="sk sk-block" />
              <div className="sk sk-block" />
            </>
          )}

          {engines.map((e, i) => {
            const fid = FIDELITY[e.fidelity] ?? { label: e.fidelity, cls: '' };
            return (
              <div className="engine-item" key={e.id} style={{ '--i': i }}>
                <span className={`dot ${e.available ? e.fidelity : 'none'}`} />
                <div className="body">
                  <div className="name">
                    {e.label}
                    <span className={`fid ${fid.cls}`}>{fid.label}</span>
                    {!e.available && <span className="fid">未安装</span>}
                  </div>
                  <div className="desc">{e.fidelity_note}</div>
                  <div className="detail">{e.detail}</div>
                  {e.available && e.apps?.length > 0 && (
                    <div className="detail">
                      可处理：{e.apps.map(appLabel).join(' / ')}
                    </div>
                  )}
                  {e.id === 'libreoffice' && !e.available && (
                    <div className="install-row">
                      <code className="install-cmd">
                        winget install --id {wingetId} -e
                      </code>
                      <button className="link-btn" onClick={copyCmd}>
                        {copied ? '已复制' : '复制命令'}
                      </button>
                      <button
                        className="link-btn"
                        onClick={() =>
                          openUrl(
                            'https://www.libreoffice.org/download/download-libreoffice/',
                          )
                        }
                      >
                        官网下载
                      </button>
                    </div>
                  )}
                </div>
              </div>
            );
          })}

          {!probing && engines.length === 0 && (
            <div className="empty-note">探测失败，请点击右上角重新检测</div>
          )}
        </div>

        <div className="sheet-foot">
          <IconShield size={14} />
          <span>
            所有转换都在本机完成。切换到别的引擎不会重新排版你的源文件——
            源文件始终以只读方式打开。
          </span>
        </div>
      </div>
    </>
  );
}

function appLabel(a) {
  return { word: 'Word', excel: 'Excel', powerpoint: 'PowerPoint' }[a] ?? a;
}
