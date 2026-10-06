import { useEffect, useRef, useState } from 'react';
import { getStore } from '@tauri-apps/plugin-store';

const STORE_FILE = 'format-forge.json';

/**
 * 会话间持久化的偏好。
 *
 * 只存真正值得记住的：输出目录、上一次的目标格式、参数面板的值。
 * 不存投放过的文件列表——用户下次大概会换一批文件。
 *
 * 加载完成前不做写入，否则会把默认值覆盖掉已存的设置。
 */
export function useSettings() {
  const [settings, setSettings] = useState({
    outputDir: '',
    target: null,
    options: {},
  });
  const [ready, setReady] = useState(false);
  const saving = useRef(false);

  useEffect(() => {
    let alive = true;
    getStore(STORE_FILE)
      .then(async (store) => {
        const saved = await store.get('settings');
        if (alive && saved && typeof saved === 'object') {
          setSettings((prev) => ({
            outputDir: saved.outputDir ?? prev.outputDir,
            target: saved.target ?? prev.target,
            options: saved.options ?? prev.options,
          }));
        }
      })
      .catch(() => {
        // 存储不可用不该拦住用户
      })
      .finally(() => {
        if (alive) setReady(true);
      });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (!ready || saving.current) return;
    saving.current = true;
    getStore(STORE_FILE)
      .then((store) => store.set('settings', settings).then(() => store.save()))
      .catch(() => {})
      .finally(() => {
        saving.current = false;
      });
  }, [ready, settings]);

  return { settings, setSettings, ready };
}
