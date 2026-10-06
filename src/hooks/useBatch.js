import { useCallback, useEffect, useRef, useState } from 'react';
import { cancelBatch, startBatch } from '../lib/api.js';

/**
 * 一次批量的生命周期。
 *
 * 作业状态放在一个按 id 索引的对象里，每次事件只替换变化的那一条。
 * FileRow 上了 memo，所以 50 行 @ 10Hz 只会重渲染真正变化的行。
 */
export function useBatch() {
  const [phase, setPhase] = useState('idle'); // idle | running | done
  const [status, setStatus] = useState({}); // id -> { state, p, error, bytesOut, bytesIn }
  const [tick, setTick] = useState(null);
  const [summary, setSummary] = useState(null);
  const [notices, setNotices] = useState([]);
  const [batchId, setBatchId] = useState(null);
  const [fatal, setFatal] = useState(null);

  // 事件回调在组件之间传递，不参与渲染；用 ref 存句柄避免闭包过期
  const batchIdRef = useRef(null);
  useEffect(() => {
    batchIdRef.current = batchId;
  }, [batchId]);

  const reset = useCallback(() => {
    setStatus({});
    setTick(null);
    setSummary(null);
    setNotices([]);
    setFatal(null);
    setPhase('idle');
    setBatchId(null);
  }, []);

  const start = useCallback(
    async (req) => {
      reset();
      setPhase('running');

      const onEvent = (ev) => {
        switch (ev.kind) {
          case 'started':
            setTick({
              done: 0,
              total: ev.total,
              failed: 0,
              bytesOut: 0,
              elapsedMs: 0,
              etaMs: null,
            });
            break;

          case 'state':
            setStatus((prev) => ({
              ...prev,
              [ev.id]: { ...prev[ev.id], state: ev.state, error: ev.error ?? null },
            }));
            break;

          case 'progress':
            setStatus((prev) => ({
              ...prev,
              [ev.id]: { ...prev[ev.id], p: ev.p },
            }));
            break;

          case 'output':
            setStatus((prev) => ({
              ...prev,
              [ev.id]: {
                ...prev[ev.id],
                bytesIn: ev.bytesIn,
                bytesOut: ev.bytesOut,
                ms: ev.ms,
                outPath: ev.dst,
              },
            }));
            break;

          case 'tick':
            setTick({
              done: ev.done,
              total: ev.total,
              failed: ev.failed,
              bytesOut: ev.bytesOut,
              elapsedMs: ev.elapsedMs,
              etaMs: ev.etaMs,
            });
            break;

          case 'notice':
            setNotices((prev) => (prev.length > 60 ? prev : [...prev, ev.message]));
            break;

          case 'finished':
            setSummary({
              ok: ev.ok,
              cancelled: ev.cancelled,
              succeeded: ev.succeeded,
              failed: ev.failed,
              bytesOut: ev.bytesOut,
              ms: ev.ms,
            });
            setPhase('done');
            break;

          default:
            break;
        }
      };

      try {
        const ack = await startBatch(req, onEvent);
        setBatchId(ack.batch_id);
        if (ack.notices?.length) setNotices((prev) => [...ack.notices, ...prev]);
        return ack;
      } catch (e) {
        setPhase('idle');
        setFatal(typeof e === 'string' ? e : (e?.message ?? '启动失败'));
        throw e;
      }
    },
    [reset],
  );

  const cancel = useCallback(async () => {
    const id = batchIdRef.current;
    if (id == null) return;
    try {
      await cancelBatch(id);
    } catch {
      // 批次可能刚好结束了，取消失败不打扰用户
    }
  }, []);

  return { phase, status, tick, summary, notices, batchId, fatal, start, cancel, reset };
}
