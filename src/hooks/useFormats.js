import { useCallback, useEffect, useState } from 'react';
import { listFormats, probeEngines } from '../lib/api.js';

/** 格式注册表。启动拉一次，之后不再变。 */
export function useFormats() {
  const [formats, setFormats] = useState(null);
  const [error, setError] = useState(null);

  useEffect(() => {
    let alive = true;
    listFormats()
      .then((f) => {
        if (alive) setFormats(f);
      })
      .catch((e) => {
        if (alive) setError(e);
      });
    return () => {
      alive = false;
    };
  }, []);

  const byId = formats ? Object.fromEntries(formats.map((f) => [f.id, f])) : {};
  return { formats, byId, loading: !formats && !error, error };
}

/** 引擎探测结果 */
export function useEngines() {
  const [report, setReport] = useState(null);
  const [probing, setProbing] = useState(true);

  // refresh 会 setState，但 setReport / setProbing(false) 都发生在 await
  // 之后，不构成同步级联渲染。oxlint 不穿 async 边界，这里是误报。
  const refresh = useCallback(async () => {
    setProbing(true);
    try {
      setReport(await probeEngines());
    } catch {
      setReport(null);
    } finally {
      setProbing(false);
    }
  }, []);

  useEffect(() => {
    // eslint-disable-next-line react/set-state-in-effect
    refresh();
  }, [refresh]);

  const best = report?.engines?.find((e) => e.available) || null;
  return { report, best, probing, refresh };
}
