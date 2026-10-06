/**
 * 组件 props 一致性检查。
 *
 * 起因：`FileQueue` 解构了 `collapsing`，但 `App.jsx` 从来没传过它，
 * 于是 `collapsing.has(...)` 抛 TypeError。项目里没有 ErrorBoundary，
 * React 一吐异常就把整棵树卸掉 —— 表现是「拖进文件后整个窗口黑屏」，
 * 而且 oxlint 与构建都发现不了。
 *
 * 这个脚本把「每个组件解构了哪些 props」与「调用点传了哪些」对一遍。
 * 少传的那个就是黑屏的根源。往组件里加 prop 而忘了在调用点补上时，
 * `npm run check:props` 会直接报出来。
 *
 * 注意它只看 App.jsx 与 FileQueue.jsx 这两处调用点，不是完整的类型检查——
 * 目的很窄：堵住这一类「传漏了」的崩溃。
 */
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const SRC = 'src';
const COMPONENTS = join(SRC, 'components');

/** 抽出 `export default function Name({ a, b }) {` 里的参数名 */
function paramsOf(code) {
  const m = code.match(/export default function \w+\(\s*\{([\s\S]*?)\}\s*,?\s*\)\s*\{/);
  if (!m) return null;
  return m[1]
    .split(',')
    .map((s) => s.split('=')[0].trim())
    .filter(Boolean);
}

/** 抽出某个调用点的 JSX 属性名，如 <FileQueue files={x} onReveal={y} /> */
function propsAt(code, name) {
  const i = code.indexOf(`<${name}`);
  if (i < 0) return null;

  // 从 <Name 扫到配对的 `>`。属性值都是 {} 表达式，数花括号就够定位了。
  let depth = 0;
  let j = i;
  for (; j < code.length; j += 1) {
    const c = code[j];
    if (c === '{') depth += 1;
    else if (c === '}') depth -= 1;
    else if (c === '>' && depth === 0) break;
  }
  return [...code.slice(i, j).matchAll(/(\w+)=\{/g)].map((m) => m[1]);
}

const app = readFileSync(join(SRC, 'App.jsx'), 'utf8');
const fileQueue = readFileSync(join(COMPONENTS, 'FileQueue.jsx'), 'utf8');

// 调用点：App.jsx 里的各个组件，加上 FileQueue → FileRow
const callSites = new Map();
for (const f of readdirSync(COMPONENTS)) {
  const name = f.replace('.jsx', '');
  if (name === 'icons') continue;
  const props = propsAt(app, name);
  if (props) callSites.set(name, props);
}
callSites.set('FileRow', propsAt(fileQueue, 'FileRow') ?? []);

let problems = 0;

for (const f of readdirSync(COMPONENTS)) {
  const name = f.replace('.jsx', '');
  if (name === 'icons') continue;

  const want = paramsOf(readFileSync(join(COMPONENTS, f), 'utf8'));
  const got = callSites.get(name);
  if (!want || !got) continue;

  const missing = want.filter((w) => !got.includes(w));
  const extra = got.filter((p) => !want.includes(p));

  if (missing.length || extra.length) {
    problems += 1;
    console.log(`${name}:`);
    if (missing.length) console.log(`   解构了但调用点没传 → ${missing.join(', ')}`);
    if (extra.length) console.log(`   调用点传了但没解构 → ${extra.join(', ')}`);
  }
}

if (problems > 0) {
  console.log(`\n${problems} 个组件的 props 对不上。`);
  process.exit(1);
}
console.log('组件 props 一致。');
