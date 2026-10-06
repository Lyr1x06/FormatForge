/**
 * 格式的表现层元数据。
 *
 * 转换能力（谁能转谁）在 Rust 的注册表里，是唯一事实来源；
 * 这里只放「长什么样」——颜色、图标字、短标签，与能力表解耦。
 */

const META = {
  png: { tint: '#7fb2ff', glyph: 'PNG' },
  jpeg: { tint: '#ffc46b', glyph: 'JPG' },
  webp: { tint: '#7fe0b0', glyph: 'WEBP' },
  avif: { tint: '#c39cff', glyph: 'AVIF' },
  gif: { tint: '#ff9d94', glyph: 'GIF' },
  bmp: { tint: '#9fb4d4', glyph: 'BMP' },
  tiff: { tint: '#d4c39f', glyph: 'TIF' },
  svg: { tint: '#ffb0e0', glyph: 'SVG' },
  ico: { tint: '#a0e0ff', glyph: 'ICO' },
  heic: { tint: '#ffd08a', glyph: 'HEIC' },
  pdf: { tint: '#ff8f86', glyph: 'PDF' },
  docx: { tint: '#6ba8ff', glyph: 'DOC' },
  doc: { tint: '#6ba8ff', glyph: 'DOC' },
  xlsx: { tint: '#7fe0a0', glyph: 'XLS' },
  xls: { tint: '#7fe0a0', glyph: 'XLS' },
  pptx: { tint: '#ffa87f', glyph: 'PPT' },
  ppt: { tint: '#ffa87f', glyph: 'PPT' },
  csv: { tint: '#9fe0a0', glyph: 'CSV' },
  tsv: { tint: '#9fe0a0', glyph: 'TSV' },
  json: { tint: '#ffd98a', glyph: 'JSON' },
  txt: { tint: '#b8c4d8', glyph: 'TXT' },
  md: { tint: '#b8c4d8', glyph: 'MD' },
  html: { tint: '#ffb08a', glyph: 'HTML' },
};

const FALLBACK = { tint: '#8a93a8', glyph: '?' };

export function meta(formatId) {
  return META[formatId] || FALLBACK;
}

/** 徽标里显示的短字。长于 4 个字符就截断。 */
export function glyph(formatId) {
  const g = meta(formatId).glyph;
  return g.length > 4 ? g.slice(0, 4) : g;
}

export function tint(formatId) {
  return meta(formatId).tint;
}

/** 大类的颜色，用于目标格式分组的圆点 */
export function categoryTint(category) {
  return { image: 'var(--accent-azure)', document: 'var(--accent-rose)', data: 'var(--accent-mint)' }[
    category
  ] || 'var(--ink-faint)';
}

export function categoryLabel(category) {
  return { image: '图片', document: '文档', data: '数据' }[category] || '其他';
}
