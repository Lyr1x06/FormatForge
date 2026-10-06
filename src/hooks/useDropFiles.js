import { useCallback, useEffect, useRef, useState } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';

/**
 * 从 Tauri 的原生拖放事件收集文件路径。
 *
 * 为什么不用 HTML5 的 drop：`dragDropEnabled: true` 时（默认），
 * WebView2 上的 DOM 拖放事件根本不会触发——Tauri 自己接管了。
 * 反过来关掉它又拿不到文件夹的真实路径。所以走原生事件。
 *
 * 原生事件给的是物理像素坐标，要除以 devicePixelRatio 才能定位光晕。
 */
export function useDropFiles(onDrop) {
  const [over, setOver] = useState(false);
  const [pos, setPos] = useState(null);
  const dropRef = useRef(onDrop);

  // 用 effect 同步回调，避免在渲染期间写 ref
  useEffect(() => {
    dropRef.current = onDrop;
  }, [onDrop]);

  useEffect(() => {
    let unlisten = null;
    let disposed = false;

    getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (payload.type === 'over') {
          setOver(true);
          const dpr = window.devicePixelRatio || 1;
          setPos({ x: payload.position.x / dpr, y: payload.position.y / dpr });
        } else if (payload.type === 'drop') {
          setOver(false);
          setPos(null);
          const paths = payload.paths ?? [];
          if (paths.length) dropRef.current?.(paths);
        } else {
          setOver(false);
          setPos(null);
        }
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const clear = useCallback(() => {
    setOver(false);
    setPos(null);
  }, []);

  return { over, pos, clear };
}
