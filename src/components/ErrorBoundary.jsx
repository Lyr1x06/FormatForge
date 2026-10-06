import { Component } from 'react';
import { IconAlert, IconRetry } from './icons.jsx';

/**
 * 渲染异常的兜底界面。
 *
 * 起因是一次真实事故：`FileQueue` 解构了一个调用点没传的 prop，
 * `undefined.has(...)` 抛 TypeError，React 卸载整棵树——用户看到的
 * 是**整个窗口变黑，没有任何线索**。排查只能靠猜。
 *
 * 拦得住：render、构造函数、生命周期，以及 **useEffect 回调**里抛出的异常
 * （实测确认过——`useDropFiles` 里 `getCurrentWebview()` 报错时就是落到这里）。
 *
 * 拦不住：事件回调（onClick 等）与 Promise / setTimeout 里的异常。
 * 这些不会让 React 卸载组件树，所以本来也不会黑屏，但要留意它们不会
 * 出现在这个界面上。
 *
 * 用 class 是因为 React 只有 class 组件能实现边界，函数组件没有替代方案。
 */
export default class ErrorBoundary extends Component {
  state = { error: null, info: null };

  static getDerivedStateFromError(error) {
    return { error };
  }

  componentDidCatch(error, info) {
    // 组件栈比错误栈更能指出是哪一层炸的，一并留着
    this.setState({ info });
    console.error('[Format Forge] 渲染失败', error, info);
  }

  render() {
    const { error, info } = this.state;
    if (!error) return this.props.children;
    return (
      <CrashReport
        error={error}
        info={info}
        onRetry={() => this.setState({ error: null, info: null })}
      />
    );
  }
}

function CrashReport({ error, info, onRetry }) {
  const title = error?.name ? `${error.name}` : '渲染失败';
  const message = error?.message || String(error) || '未知错误';
  const detail = buildDetail(error, info);

  const copy = () => {
    navigator.clipboard?.writeText(detail).catch(() => {});
  };

  return (
    <div className="crash">
      <div className="crash-card card">
        <div className="crash-head">
          <span className="crash-icon">
            <IconAlert size={17} />
          </span>
          <div>
            <h2>{title}</h2>
            <p className="crash-msg">{message}</p>
          </div>
        </div>

        <details className="crash-detail">
          <summary>技术细节</summary>
          {/* 全局把 user-select 关了，这里要单独放开，否则复制不走 */}
          <pre>{detail}</pre>
        </details>

        <div className="crash-actions">
          <button className="btn-ghost" onClick={copy}>
            复制错误信息
          </button>
          <button className="btn-ghost" onClick={onRetry}>
            <IconRetry size={14} />
            重试
          </button>
          <button className="btn-primary" onClick={() => window.location.reload()}>
            重新加载
          </button>
        </div>

        <p className="crash-hint">
          「重试」会就地重挂组件树，文件列表会清空；「重新加载」整页重启。
          如果反复出现，把上面的错误信息发出来。
        </p>
      </div>
    </div>
  );
}

/** 把错误与组件栈拼成一段可以直接贴出来的文本 */
function buildDetail(error, info) {
  const parts = [];
  if (error?.stack) parts.push(error.stack);
  else if (error) parts.push(String(error));
  if (info?.componentStack) parts.push(`\n组件栈：${info.componentStack}`);
  parts.push(`\n时间：${new Date().toISOString()}`);
  return parts.join('\n');
}
