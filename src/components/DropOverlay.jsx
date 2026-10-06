import { IconLayers } from './icons.jsx';

/**
 * 全窗口拖入遮罩。
 *
 * 始终挂载，靠 opacity 过渡切换——这样淡出不需要「延迟卸载」那套
 * state 计时逻辑，拖入/拖出只是改一个类名。
 *
 * 光晕跟随光标用 90ms linear 过渡，刻意不用缓动：
 * 弹簧会让光晕明显落后于指针，那读起来像卡顿而不是灵动。
 */
export default function DropOverlay({ active, pos }) {
  return (
    <div className={`drop-overlay${active ? ' on' : ''}`} aria-hidden={!active}>
      {pos && (
        <div className="cursor-glow" style={{ left: `${pos.x}px`, top: `${pos.y}px` }} />
      )}
      <div className="frame">
        <IconLayers size={30} />
        <div className="pill">松手即添加到队列</div>
        <div className="sub">支持文件夹，会自动递归展开</div>
      </div>
    </div>
  );
}
