/**
 * 细进度环。
 *
 * 用 stroke-dashoffset 过渡而不是 width——这个技巧和 aurora-lens 里
 * 画轨迹用的是同一套，圆环的视觉推进比直条更「软」。
 *
 * 尺寸固定 22px，所以圆周是常数，不必每次测量。
 */
const R = 8.5;
const C = 2 * Math.PI * R;

export default function ProgressRing({ p = 0, done = false, indeterminate = false }) {
  const offset = C * (1 - Math.max(0, Math.min(1, p)));

  return (
    <svg
      className={`ring${done ? ' is-done' : ''}${indeterminate ? ' indeterminate' : ''}`}
      width="22"
      height="22"
      viewBox="0 0 22 22"
      aria-hidden="true"
    >
      <circle className="track" cx="11" cy="11" r={R} fill="none" strokeWidth="2.4" />
      <circle
        className="fill"
        cx="11"
        cy="11"
        r={R}
        fill="none"
        strokeWidth="2.4"
        strokeDasharray={C}
        strokeDashoffset={indeterminate ? C * 0.78 : offset}
        transform="rotate(-90 11 11)"
      />
    </svg>
  );
}
