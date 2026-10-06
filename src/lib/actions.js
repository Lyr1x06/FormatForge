/**
 * 动作型目标的表现层元数据。
 *
 * 注册表里的目标有两种：**格式**（`csv`、`webp`）和**动作**（`@split`、`@merge`）。
 * 动作产出的还是一个格式，只是需要额外参数，并且有些是 N→1 或 1→N 的形状，
 * 不能套用「每个文件出一份」的默认假设。
 *
 * 与格式元数据一样，这里只放「长什么样」，能力与参数都在 Rust 注册表里。
 */
const ACTIONS = {
  '@organize': {
    label: '整理页面',
    hint: '旋转、删除、重排，产出一个 PDF',
    shape: 'one-to-one',
    // 参数定义挂在哪个格式上——动作本身不另起一张参数表
    owner: 'pdf',
    tint: '#82b8ff',
    glyph: '整理',
  },
  '@split': {
    label: '拆分',
    hint: '按页码范围或每 N 页切出多个 PDF',
    shape: 'one-to-many',
    owner: 'pdf',
    tint: '#7fe0b0',
    glyph: '拆分',
  },
  '@merge': {
    label: '合并为一个 PDF',
    hint: '把这一批 PDF 按添加顺序拼成一个',
    shape: 'many-to-one',
    owner: 'pdf',
    tint: '#ffc46b',
    glyph: '合并',
  },
};

export function isAction(id) {
  return typeof id === 'string' && id.startsWith('@');
}

export function action(id) {
  return ACTIONS[id] ?? null;
}
