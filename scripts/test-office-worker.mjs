/**
 * Office worker 命令行测试台。
 * 用法：node scripts/test-office-worker.mjs <word|excel|powerpoint> <输出目录> <输入文件...>
 *
 * 直接跑 worker 的协议，用于在没有 UI 的情况下验证 COM 转换与保真度。
 */
import { spawn } from 'node:child_process';
import path from 'node:path';
import { mkdirSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const WORKER = path.join(ROOT, 'src-tauri', 'scripts', 'office-worker.ps1');

const [app, outDirArg, ...inputs] = process.argv.slice(2);
if (!app || !outDirArg || inputs.length === 0) {
  console.error('用法: node scripts/test-office-worker.mjs <word|excel|powerpoint> <输出目录> <输入文件...>');
  process.exit(2);
}

const outDir = path.resolve(outDirArg);
if (!existsSync(outDir)) mkdirSync(outDir, { recursive: true });

const jobs = inputs.map((src) => {
  const abs = path.resolve(src);
  const stem = path.basename(abs, path.extname(abs));
  return { src: abs, dst: path.join(outDir, `${stem}.pdf`) };
});

console.log(`应用 ${app} · ${jobs.length} 个文件 · 输出到 ${outDir}\n`);

const p = spawn(
  'powershell',
  ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', WORKER, '-App', app],
  { stdio: ['pipe', 'pipe', 'pipe'] },
);

let buf = '';
let qi = 0;
const t0 = Date.now();
let tPrev = t0;

function next() {
  if (qi < jobs.length) {
    p.stdin.write(`${JSON.stringify({ op: 'convert', src: jobs[qi].src, dst: jobs[qi].dst })}\n`);
  } else {
    p.stdin.write(`${JSON.stringify({ op: 'quit' })}\n`);
  }
}

p.stdout.on('data', (d) => {
  buf += d.toString('utf8');
  const lines = buf.split('\n');
  buf = lines.pop() ?? '';
  for (const line of lines) {
    const t = line.trim();
    if (!t) continue;
    if (!t.startsWith('@@FF@@')) {
      console.log(`  [噪声] ${t.slice(0, 120)}`);
      continue;
    }
    let m;
    try {
      m = JSON.parse(t.slice(6));
    } catch {
      console.log(`  [协议错误] ${t.slice(0, 160)}`);
      continue;
    }

    if (m.op === 'ready') {
      console.log(`  就绪 @ ${Date.now() - t0}ms   （COM 冷启动，整批只付这一次）`);
      next();
    } else if (m.op === 'ok') {
      const dt = Date.now() - tPrev;
      tPrev = Date.now();
      qi++;
      console.log(`  ✓  ${path.basename(m.src)}  →  ${path.basename(m.dst)}   ${dt}ms`);
      next();
    } else if (m.op === 'err') {
      qi++;
      console.log(`  ✗  ${path.basename(m.src)}   HRESULT=${m.hresult}  ${m.msg}`);
      next();
    } else if (m.op === 'fatal') {
      console.log(`  FATAL ${JSON.stringify(m)}`);
      p.kill();
    }
  }
});

p.stderr.on('data', (d) => process.stderr.write(d.toString('utf8')));
p.on('exit', (c) => {
  console.log(`\n[退出码 ${c}]  总耗时 ${Date.now() - t0}ms`);
});
