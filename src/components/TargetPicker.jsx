import { categoryLabel, categoryTint, glyph, tint } from '../lib/format-meta.js';
import { action, isAction } from '../lib/actions.js';
import { countFor, labelFor } from '../lib/targets.js';

/**
 * 目标选单。
 *
 * 选项来自当前所选文件的合法目标交集，所以不合法组合在这里是不可表达的，
 * 不需要「置灰」这种会误导人的处理。
 *
 * 两类目标混在同一个选单里：
 *   * **格式**（`csv`、`webp`）——每个文件出一份
 *   * **动作**（`@split`、`@merge`）——形状不同，芯片上要额外说明
 *
 * 每个芯片上的数字是「有多少个文件会参与」，而不是「会产出多少份」——
 * 合并和拆分都会打破这个对应关系，所以芯片的 title 里写明形状。
 */
export default function TargetPicker({ targets, byId, value, onChange, files, disabled }) {
  const groups = groupOf(targets, byId);

  return (
    <div className="target-groups">
      {groups.map((g) => (
        <div className="target-group" key={g.key}>
          <div className="group-label">
            <span className="cat-dot" style={{ background: g.tint }} />
            {g.label}
          </div>
          <div className="target-row">
            {g.items.map((id) => {
              const n = countFor(id, files);
              const act = isAction(id);
              const label = labelFor(id, byId);
              const hint = act ? action(id).hint : null;

              return (
                <button
                  key={id}
                  className={`target-chip${value === id ? ' active' : ''}${act ? ' is-action' : ''}`}
                  onClick={() => onChange(id)}
                  disabled={disabled || n === 0}
                  title={hint ? `${hint}（${n} 个文件）` : `${n} 个文件可转为 ${label}`}
                >
                  <span
                    className="chip-glyph"
                    style={{ color: value === id ? undefined : act ? action(id).tint : tint(id) }}
                  >
                    {act ? action(id).glyph : glyph(id)}
                  </span>
                  {label}
                  <span className="chip-count">{n}</span>
                </button>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}

/** 按「格式类别」与「动作」分组，动作单独成一组放在最后 */
function groupOf(targets, byId) {
  const groups = [];

  for (const id of targets) {
    if (isAction(id)) {
      let g = groups.find((x) => x.key === '@action');
      if (!g) {
        g = { key: '@action', label: 'PDF 操作', tint: 'var(--accent-gold)', items: [] };
        groups.push(g);
      }
      g.items.push(id);
      continue;
    }

    const cat = byId[id]?.category ?? 'other';
    let g = groups.find((x) => x.key === cat);
    if (!g) {
      g = { key: cat, label: categoryLabel(cat), tint: categoryTint(cat), items: [] };
      groups.push(g);
    }
    g.items.push(id);
  }

  // 动作组永远排在最后——用户最常做的是换格式，动作是进阶用法
  groups.sort((a, b) => (a.key === '@action' ? 1 : b.key === '@action' ? -1 : 0));
  return groups;
}
