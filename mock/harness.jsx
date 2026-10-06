/**
 * 浏览器里的渲染验证台（开发用，不参与打包）。
 *
 * 起法：npx vite --config vite.mock.config.js  然后开 http://localhost:5210
 *   /            正常界面（假数据）
 *   /?scene=crash 故意抛一个异常，用来看错误边界长什么样
 *
 * Tauri 的 invoke 不经过 fetch，而是直接调 `window.__TAURI_INTERNALS__`，
 * 所以这里给那个对象装一个假实现，而不是替换 fetch。
 */
import { createElement as h } from 'react';
import { createRoot } from 'react-dom/client';
import ErrorBoundary from '../src/components/ErrorBoundary.jsx';
import App from '../src/App.jsx';
import formats from './formats.json';
import '../src/styles/global.css';

/* ---------- 假的 Tauri 运行时 ---------- */

let nextCb = 1;
const callbacks = new Map();

const rnd = (n) => Math.floor(Math.random() * n);

function makeScan() {
  const spec = [
    ['报告.docx', 'docx'],
    ['台账.xlsx', 'xlsx'],
    ['照片.png', 'png'],
    ['合同.pdf', 'pdf'],
  ];
  const files = spec.map(([name, fmt], i) => {
    const def = formats.find((f) => f.id === fmt);
    return {
      id: i,
      path: `D:\\demo\\${name}`,
      name,
      rel: null,
      bytes: 1024 * (10 + rnd(4000)),
      format: fmt,
      format_label: def?.label ?? fmt,
      category: def?.category ?? 'document',
      mismatch: false,
      targets: def?.targets ?? [],
      unsupported: false,
    };
  });
  const common = files.map((f) => f.targets).reduce((a, c) => a.filter((t) => c.includes(t)));
  return {
    files,
    skipped: [],
    common_targets: common,
    total_bytes: files.reduce((s, f) => s + f.bytes, 0),
    truncated: false,
    suggested_output_dir: 'D:\\demo\\out',
  };
}

const MOCK = {
  list_formats: () => formats,
  scan_inputs: () => makeScan(),
  probe_engines: () => ({
    engines: [
      {
        id: 'office',
        label: 'Microsoft Office',
        available: true,
        fidelity: 'exact',
        apps: [],
        detail: '验证台的假数据',
      },
    ],
    recommended: 'office',
    libreoffice_winget_id: 'TheDocumentFoundation.LibreOffice',
  }),
  start_batch: () => ({ batch_id: 1, notices: [] }),
  cancel_batch: () => null,
  clear_engine_failure: () => null,
  open_url: () => null,
};

window.__TAURI_INTERNALS__ = {
  invoke: async (cmd) => {
    // 对话框：假装用户选了 D:\demo
    if (cmd === 'plugin:dialog|open') return 'D:\\demo';
    // 设置存储：不落盘，一律当空
    if (cmd.startsWith('plugin:store|')) {
      if (cmd.endsWith('|get')) return null;
      return null;
    }
    const fn = MOCK[cmd];
    if (!fn) throw new Error(`验证台没有为 "${cmd}" 准备假数据`);
    return fn();
  },
  transformCallback: (cb) => {
    const id = nextCb++;
    callbacks.set(id, cb);
    return id;
  },
  unregisterCallback: (id) => callbacks.delete(id),
  convertFileSrc: (p) => p,
};

/* ---------- 两个场景 ---------- */

function Boom() {
  // 复现真实事故：解构了调用点没传的 prop
  const collapsing = undefined;
  return h('div', null, collapsing.has(1));
}

// 一直炸，直到外部显式放行 —— 用来验证「重试」确实重挂了组件树。
// （不能只炸一次：React 开发模式会把渲染跑两遍，第二次就自己好了。）
let allowed = false;
window.__allowRecover = () => {
  allowed = true;
};
function BoomUntilAllowed() {
  if (!allowed) throw new TypeError('渲染失败，等你点重试');
  return h('div', { id: 'recovered' }, '已经恢复');
}

const scene = new URLSearchParams(location.search).get('scene');
const pick = { crash: Boom, once: BoomUntilAllowed };
const Body = pick[scene] ?? App;

createRoot(document.getElementById('root')).render(h(ErrorBoundary, null, h(Body)));
