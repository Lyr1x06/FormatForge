/** 字节数 → 人类可读 */
export function bytes(n) {
  if (!Number.isFinite(n) || n < 0) return '—';
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

/** 毫秒 → 时长。短于 1 秒只显示秒。 */
export function duration(ms) {
  if (!Number.isFinite(ms) || ms < 0) return '—';
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)} 秒`;
  const m = Math.floor(s / 60);
  const rest = Math.round(s % 60);
  return `${m} 分 ${rest} 秒`;
}

/** 剩余时间。位数少时才给精确值，避免一秒钟跳好几个数字。 */
export function eta(ms) {
  if (ms == null) return null;
  const s = ms / 1000;
  if (s < 5) return '即将完成';
  if (s < 60) return `约 ${Math.round(s)} 秒`;
  if (s < 3600) return `约 ${Math.ceil(s / 60)} 分钟`;
  return `约 ${(s / 3600).toFixed(1)} 小时`;
}

/** 体积变化百分比，带方向 */
export function sizeDelta(before, after) {
  if (!before || !after) return null;
  const ratio = (after - before) / before;
  return {
    text: `${ratio < 0 ? '减少' : '增加'} ${Math.abs(Math.round(ratio * 100))}%`,
    smaller: ratio < 0,
    ratio,
  };
}

/** 0..1 → 百分比整数 */
export const pct = (p) => `${Math.round(Math.max(0, Math.min(1, p)) * 100)}%`;

/** 截断过长的中间路径，保留头尾 */
export function ellipsisPath(p, max = 52) {
  if (!p || p.length <= max) return p || '';
  const head = Math.ceil((max - 1) / 2);
  const tail = Math.floor((max - 1) / 2);
  return `${p.slice(0, head)}…${p.slice(-tail)}`;
}
