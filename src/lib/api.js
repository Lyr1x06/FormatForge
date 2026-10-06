import { invoke, Channel } from '@tauri-apps/api/core';

/** 引擎探测 */
export const probeEngines = () => invoke('probe_engines');

/** 用系统浏览器打开链接 */
export const openUrl = (url) => invoke('open_url', { url });

/** 格式注册表 */
export const listFormats = () => invoke('list_formats');

/** 扫描投放的路径，展开文件夹并识别真实格式 */
export const scanInputs = (paths, recurse = true, maxDepth = 8) =>
  invoke('scan_inputs', { req: { paths, recurse, max_depth: maxDepth } });

/** 开始转换。onEvent 收到的是 JobEvent 流。 */
export function startBatch(req, onEvent) {
  const onEventArgs = new Channel();
  onEventArgs.onmessage = onEvent;
  return invoke('start_batch', { req, onEvent: onEventArgs });
}

export const cancelBatch = (batchId) => invoke('cancel_batch', { batchId });

export const clearEngineFailure = (srcFormat) =>
  invoke('clear_engine_failure', { srcFormat });

/** 把后端抛出的错误统一成一句话，界面上直接显示 */
export function errorText(e) {
  if (typeof e === 'string') return e;
  if (e?.message) return e.message;
  return '操作失败';
}
