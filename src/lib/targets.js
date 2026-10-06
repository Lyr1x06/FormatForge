import { isAction, action } from './actions.js';

/**
 * 从格式注册表里解析出「某个目标需要哪些参数」。
 *
 * 目标是格式时，参数挂在它自己的定义上；目标是动作（`@split`）时，
 * 参数挂在**它产出的那个格式**上——动作只是给那个格式补一组预设参数，
 * 不该另起一张表。这个归属写在 `actions.js` 的 `owner` 里。
 */
export function optionsFor(target, byId) {
  const ownerId = isAction(target) ? action(target)?.owner : target;
  const def = ownerId ? byId[ownerId] : null;
  if (!def) return [];

  return def.options.filter((o) => {
    // 有 targets 限定的，只在列出的目标下出现
    if (o.targets?.length) return o.targets.includes(target);
    // 没限定的对所有目标都适用——但动作目标例外，
    // 否则「渲染精度」这种格式专属参数会漏到「合并」下面
    return !isAction(target);
  });
}

/** 目标对应的默认参数值 */
export function defaultsFor(target, byId) {
  const out = {};
  for (const o of optionsFor(target, byId)) {
    if (out[o.key] === undefined) out[o.key] = o.default;
  }
  return out;
}

/**
 * 目标的展示名。格式走注册表，动作走 actions 元数据。
 * `byId` 里查不到且不是动作时退回原 id。
 */
export function labelFor(target, byId) {
  if (isAction(target)) return action(target)?.label ?? target;
  return byId[target]?.label ?? target;
}

/**
 * 一批文件里有多少个能参与这个目标。
 *
 * 合并是 N→1：少于两个文件时这个目标根本不可用，返回 0 让选单禁用掉，
 * 而不是让用户点了之后在开跑时才收到「合并需要至少两个 PDF」。
 */
export function countFor(target, files) {
  const usable = files.filter((f) => !f.unsupported && f.targets.includes(target));
  if (isAction(target) && action(target)?.shape === 'many-to-one') {
    return usable.length >= 2 ? usable.length : 0;
  }
  return usable.length;
}
