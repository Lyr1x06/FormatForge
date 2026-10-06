import { IconChevronDown } from './icons.jsx';

/**
 * 参数面板。完全由后端注册表里的 OptionDef 驱动，
 * 任何格式对都不在这里硬编码表单——加一个新格式不需要改这个文件。
 *
 * 序列化注意：Rust 的 serde 默认把枚举变体输出成 `"kind":"Slider"` 这种
 * PascalCase，字段名则是 snake_case，所以下面按那个形状读取。
 */
export default function OptionPanel({ defs, values, onChange, disabled }) {
  if (!defs.length) return null;

  return (
    <div className="options">
      {defs.map((def) => {
        const value = values[def.key] ?? def.default;
        return (
          <div className={`opt${def.kind === 'Text' ? ' wide' : ''}`} key={def.key}>
            <label className="opt-head" htmlFor={`opt-${def.key}`}>
              {def.label}
              {def.kind === 'Slider' && (
                <span className="val">
                  {value}
                  {def.unit || ''}
                </span>
              )}
              {def.kind === 'Number' && def.unit && <span className="val">{def.unit}</span>}
            </label>
            <Control
              def={def}
              value={value}
              disabled={disabled}
              onChange={(v) => onChange(def.key, v)}
            />
            {def.hint && <div className="opt-hint">{def.hint}</div>}
          </div>
        );
      })}
    </div>
  );
}

function Control({ def, value, onChange, disabled }) {
  const id = `opt-${def.key}`;

  switch (def.kind) {
    case 'Slider':
      return (
        <input
          id={id}
          type="range"
          min={def.min ?? 0}
          max={def.max ?? 100}
          step={def.step ?? 1}
          value={value ?? def.min ?? 0}
          disabled={disabled}
          onChange={(e) => onChange(Number(e.target.value))}
        />
      );

    case 'Number':
      return (
        <input
          id={id}
          type="number"
          min={def.min}
          max={def.max}
          step={def.step ?? 1}
          value={value ?? ''}
          disabled={disabled}
          onChange={(e) => {
            const n = Number(e.target.value);
            onChange(Number.isFinite(n) ? n : def.min ?? 0);
          }}
        />
      );

    case 'Toggle':
      return (
        <button
          type="button"
          className={`switch${value ? ' on' : ''}`}
          onClick={() => !disabled && onChange(!value)}
          disabled={disabled}
          aria-pressed={!!value}
        >
          <span className="knob" />
          <span className="sw-label">{value ? '开启' : '关闭'}</span>
        </button>
      );

    case 'Text':
      return (
        <input
          id={id}
          type="text"
          value={value ?? ''}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
        />
      );

    case 'Select':
    default: {
      const choices = def.choices ?? [];
      // 选项少时用分段控件，比下拉更少一次点击
      if (choices.length <= 3) {
        return (
          <div className="seg-ctl" role="group">
            {choices.map((c) => (
              <button
                key={c.value}
                type="button"
                className={value === c.value ? 'active' : ''}
                disabled={disabled}
                onClick={() => onChange(c.value)}
              >
                {c.label}
              </button>
            ))}
          </div>
        );
      }
      return (
        <div className="select-wrap">
          <select
            id={id}
            value={value ?? ''}
            disabled={disabled}
            onChange={(e) => onChange(e.target.value)}
          >
            {choices.map((c) => (
              <option key={c.value} value={c.value}>
                {c.label}
              </option>
            ))}
          </select>
          <IconChevronDown size={13} />
        </div>
      );
    }
  }
}
