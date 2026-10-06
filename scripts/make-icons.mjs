/**
 * 生成应用图标（零依赖，手写 PNG/ICO 编码）。
 * 用法：node scripts/make-icons.mjs
 * 输出到 src-tauri/icons/。
 */
import { deflateSync } from 'node:zlib';
import { writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = resolve(ROOT, 'src-tauri/icons');

/* ---------- PNG 编码 ---------- */
const CRC_TABLE = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();

function crc32(buf) {
  let c = -1;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'latin1'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

/** rgba: Uint8Array 长度 w*h*4 */
function encodePng(w, h, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;   // bit depth
  ihdr[9] = 6;   // color type RGBA
  // 10,11,12 = compression / filter / interlace，全 0

  const stride = w * 4;
  const raw = Buffer.alloc((stride + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (stride + 1)] = 0; // filter type: None
    Buffer.from(rgba.buffer, rgba.byteOffset + y * stride, stride).copy(raw, y * (stride + 1) + 1);
  }

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/* ---------- 图标绘制 ---------- */
const lerp = (a, b, t) => a + (b - a) * t;
const clamp = (v, a, b) => (v < a ? a : v > b ? b : v);

function mix(c1, c2, t) {
  return [lerp(c1[0], c2[0], t), lerp(c1[1], c2[1], t), lerp(c1[2], c2[2], t)];
}

/** 圆角矩形内部测试 */
function inRoundRect(x, y, x0, y0, x1, y1, r) {
  if (x < x0 || x > x1 || y < y0 || y > y1) return false;
  const cx = clamp(x, x0 + r, x1 - r);
  const cy = clamp(y, y0 + r, y1 - r);
  const dx = x - cx;
  const dy = y - cy;
  return dx * dx + dy * dy <= r * r;
}

/** 三角形内部测试（符号法） */
function inTriangle(px, py, [ax, ay], [bx, by], [cx, cy]) {
  const d1 = (px - bx) * (ay - by) - (ax - bx) * (py - by);
  const d2 = (px - cx) * (by - cy) - (bx - cx) * (py - cy);
  const d3 = (px - ax) * (cy - ay) - (cx - ax) * (py - ay);
  const neg = d1 < 0 || d2 < 0 || d3 < 0;
  const pos = d1 > 0 || d2 > 0 || d3 > 0;
  return !(neg && pos);
}

/**
 * 采样一点的颜色。返回 [r,g,b,a]（a 为 0..1 覆盖度）。
 * 图形针对 512×512 设计，坐标按 S 缩放。
 */
function sample(x, y, S) {
  const u = x / S;
  const v = y / S;

  // 背景：纵深渐变 + 圆角
  if (!inRoundRect(u, v, 0, 0, 512, 512, 116)) return [0, 0, 0, 0];
  const bgT = clamp(v / 512, 0, 1);
  let col = mix([20, 29, 63], [6, 9, 22], bgT);

  // 顶部内侧高光，呼应玻璃质感
  const glow = clamp(1 - v / 210, 0, 1) ** 2 * 0.16;
  col = mix(col, [130, 184, 255], glow);

  // 上方箭头（右向，金调）
  const topBar = inRoundRect(u, v, 150, 200, 332, 228, 14);
  const topHead = inTriangle(u, v, [330, 180], [392, 214], [330, 248]);
  if (topBar || topHead) {
    col = mix([255, 196, 107], [255, 157, 77], clamp((u - 150) / 242, 0, 1));
  }

  // 下方箭头（左向，天蓝）
  const botBar = inRoundRect(u, v, 180, 284, 362, 312, 14);
  const botHead = inTriangle(u, v, [182, 264], [120, 298], [182, 332]);
  if (botBar || botHead) {
    col = mix([130, 184, 255], [106, 168, 255], clamp((362 - u) / 242, 0, 1));
  }

  return [col[0], col[1], col[2], 1];
}

const SS = 4; // 超采样倍率，做抗锯齿
function render(size) {
  const S = 512 / size;
  const out = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let r = 0, g = 0, b = 0, a = 0;
      for (let sy = 0; sy < SS; sy++) {
        for (let sx = 0; sx < SS; sx++) {
          const [pr, pg, pb, pa] = sample(x * S + (sx + 0.5) * (S / SS), y * S + (sy + 0.5) * (S / SS), 1);
          r += pr * pa; g += pg * pa; b += pb * pa; a += pa;
        }
      }
      const n = SS * SS;
      const i = (y * size + x) * 4;
      if (a > 0) {
        out[i] = Math.round(r / a);
        out[i + 1] = Math.round(g / a);
        out[i + 2] = Math.round(b / a);
      }
      out[i + 3] = Math.round((a / n) * 255);
    }
  }
  return encodePng(size, size, out);
}

/** 单张 256×256 PNG 的 ICO 容器 */
function encodeIco(png, size) {
  const dir = Buffer.alloc(6);
  dir.writeUInt16LE(0, 0);
  dir.writeUInt16LE(1, 2); // type: icon
  dir.writeUInt16LE(1, 4); // count

  const entry = Buffer.alloc(16);
  entry[0] = size >= 256 ? 0 : size; // 0 表示 256
  entry[1] = size >= 256 ? 0 : size;
  entry[2] = 0; // 调色板数
  entry[3] = 0;
  entry.writeUInt16LE(1, 4);  // planes
  entry.writeUInt16LE(32, 6); // bpp
  entry.writeUInt32LE(png.length, 8);
  entry.writeUInt32LE(22, 12); // 偏移 = 6 + 16

  return Buffer.concat([dir, entry, png]);
}

/* ---------- 输出 ---------- */
if (!existsSync(OUT)) mkdirSync(OUT, { recursive: true });

const targets = [
  ['icon.png', 512],
  ['128x128@2x.png', 256],
  ['128x128.png', 128],
  ['32x32.png', 32],
];

for (const [name, size] of targets) {
  writeFileSync(resolve(OUT, name), render(size));
  console.log(`  ${name}  ${size}×${size}`);
}

writeFileSync(resolve(OUT, 'icon.ico'), encodeIco(render(256), 256));
console.log('  icon.ico  256×256');
