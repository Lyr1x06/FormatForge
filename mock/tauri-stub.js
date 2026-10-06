/**
 * 浏览器里没有 Tauri 运行时，`getCurrentWebview()` 会去读
 * `window.__TAURI_INTERNALS__.metadata` 而抛异常。这里给个空实现，
 * 让 App 在浏览器里也能挂载（拖放功能在这个验证台里不可用，也不需要）。
 */
export function getCurrentWebview() {
  return { onDragDropEvent: async () => () => {} };
}
export function getCurrentWindow() {
  return { onDragDropEvent: async () => () => {} };
}
