import { useCallback, useEffect, useMemo, useState } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { revealItemInDir } from '@tauri-apps/plugin-opener';

import { errorText, scanInputs } from './lib/api.js';
import { defaultsFor, labelFor, optionsFor, countFor } from './lib/targets.js';
import { useDropFiles } from './hooks/useDropFiles.js';
import { useBatch } from './hooks/useBatch.js';
import { useEngines, useFormats } from './hooks/useFormats.js';
import { useSettings } from './hooks/useSettings.js';

import TopBar from './components/TopBar.jsx';
import DropZone from './components/DropZone.jsx';
import DropOverlay from './components/DropOverlay.jsx';
import FileQueue from './components/FileQueue.jsx';
import TargetPicker from './components/TargetPicker.jsx';
import OptionPanel from './components/OptionPanel.jsx';
import ConvertBar from './components/ConvertBar.jsx';
import EngineSheet from './components/EngineSheet.jsx';

export default function App() {
  const { byId, loading: formatsLoading, error: formatsError } = useFormats();
  const { report: engines, best, probing, refresh: refreshEngines } = useEngines();
  const batch = useBatch();
  const { settings, setSettings, ready: settingsReady } = useSettings();

  const [scan, setScan] = useState(null);
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState(null);
  const [sheetOpen, setSheetOpen] = useState(false);

  const { outputDir, target, options } = settings;

  const setOutputDir = useCallback(
    (dir) => setSettings((s) => ({ ...s, outputDir: dir })),
    [setSettings],
  );
  const setTarget = useCallback(
    (id) => setSettings((s) => ({ ...s, target: id })),
    [setSettings],
  );

  const files = useMemo(() => scan?.files ?? [], [scan]);
  const commonTargets = useMemo(() => scan?.common_targets ?? [], [scan]);

  /* ---------- 输出目录 ---------- */

  const pickOutputDir = useCallback(async () => {
    try {
      const picked = await openDialog({ directory: true, multiple: false });
      if (typeof picked === 'string') setOutputDir(picked);
    } catch {
      // 用户取消
    }
  }, [setOutputDir]);

  /* ---------- 扫描 ---------- */

  const doScan = useCallback(
    async (paths) => {
      if (!paths?.length) return;
      setScanning(true);
      setScanError(null);
      batch.reset();

      // 还没有输出目录就先问一次，免得转完才发现不知道文件在哪
      let dir = outputDir;
      if (!dir) {
        try {
          const picked = await openDialog({
            directory: true,
            multiple: false,
            title: '选择输出目录',
          });
          if (typeof picked !== 'string') {
            setScanning(false);
            return;
          }
          dir = picked;
          setOutputDir(picked);
        } catch {
          setScanning(false);
          return;
        }
      }

      try {
        const result = await scanInputs(paths);
        setScan(result);

        const common = result.common_targets ?? [];
        setTarget((prev) => (prev && common.includes(prev) ? prev : (common[0] ?? null)));

        if (!result.files.length) setScanError('没有找到可转换的文件');
      } catch (e) {
        setScanError(errorText(e));
      } finally {
        setScanning(false);
      }
    },
    [batch, outputDir, setTarget, setOutputDir],
  );

  const { over, pos } = useDropFiles(doScan);

  const pickFiles = useCallback(async () => {
    try {
      const picked = await openDialog({ multiple: true, directory: false });
      if (picked) doScan(Array.isArray(picked) ? picked : [picked]);
    } catch (e) {
      setScanError(errorText(e));
    }
  }, [doScan]);

  const pickFolder = useCallback(async () => {
    try {
      const picked = await openDialog({ directory: true, multiple: false });
      if (picked) doScan([picked]);
    } catch (e) {
      setScanError(errorText(e));
    }
  }, [doScan]);

  /* ---------- 参数默认值 ---------- */

  // 目标是格式或动作，参数面板都从注册表推导（动作的参数挂在它产出的格式上）
  useEffect(() => {
    if (!target) return;
    const defaults = defaultsFor(target, byId);
    setSettings((s) => {
      const next = { ...s.options };
      let changed = false;
      for (const [k, v] of Object.entries(defaults)) {
        if (next[k] === undefined) {
          next[k] = v;
          changed = true;
        }
      }
      return changed ? { ...s, options: next } : s;
    });
  }, [target, byId, setSettings]);

  const activeOptions = useMemo(() => optionsFor(target, byId), [target, byId]);

  /* ---------- 移除行 ---------- */

  // 正在折叠的行。删除不是立刻把行从数组里拿掉——先把它的高度收成 0，
  // 动画走完再真删。否则行会「啪」地消失，队列瞬间上跳。
  const [collapsing, setCollapsing] = useState(() => new Set());

  const removeFile = useCallback((id) => {
    setCollapsing((prev) => new Set(prev).add(id));
    setTimeout(() => {
      setScan((prev) => {
        if (!prev) return prev;
        const nextFiles = prev.files.filter((f) => f.id !== id);
        return { ...prev, files: nextFiles, common_targets: intersectTargets(nextFiles) };
      });
      setCollapsing((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }, 320); // 与 .row-wrap 的过渡时长一致
  }, []);

  // 移除文件后目标格式可能已经不合法，及时纠正
  useEffect(() => {
    if (!scan) return;
    if (target && commonTargets.includes(target)) return;
    setTarget(commonTargets[0] ?? null);
  }, [scan, target, commonTargets, setTarget]);

  const clearAll = useCallback(() => {
    setScan(null);
    setScanError(null);
    batch.reset();
  }, [batch]);

  /* ---------- 开跑 ---------- */

  const runnable = useMemo(
    () => files.filter((f) => !f.unsupported && (!target || f.targets.includes(target))),
    [files, target],
  );

  // 合并这种 N→1 的动作，少于两个文件就不可用——用 countFor 而不是
  // runnable.length，否则单文件时「开始转换」会亮着，点了才发现白跑一趟。
  const canRun =
    batch.phase === 'idle' &&
    !!target &&
    !!outputDir &&
    countFor(target, files) > 0 &&
    settingsReady;

  const run = useCallback(async () => {
    if (!canRun) return;
    try {
      await batch.start({
        files: files.filter((f) => !f.unsupported),
        dst_format: target,
        output_dir: outputDir,
        options,
      });
    } catch (e) {
      setScanError(errorText(e));
    }
  }, [canRun, batch, files, target, outputDir, options]);

  const openOutput = useCallback(async () => {
    if (!outputDir) return;
    try {
      await revealItemInDir(outputDir);
    } catch {
      // 目录可能已被移走
    }
  }, [outputDir]);

  const reveal = useCallback(async (path) => {
    try {
      await revealItemInDir(path);
    } catch {
      // 忽略
    }
  }, []);

  /* ---------- 渲染 ---------- */

  const showQueue = files.length > 0;

  return (
    <div className="app">
      <div className="ambient" />
      <div className="ambient-noise" />

      <div className="shell">
        <TopBar
          engine={best}
          probing={probing}
          outputDir={outputDir}
          onPickOutput={pickOutputDir}
          onShowEngines={() => setSheetOpen(true)}
          running={batch.phase === 'running'}
        />

        <div className="stage">
          {!showQueue ? (
            <DropZone
              onPickFiles={pickFiles}
              onPickFolder={pickFolder}
              scanning={scanning}
              error={scanError ?? (formatsError ? '无法读取格式注册表' : null)}
              formats={byId}
              formatsLoading={formatsLoading}
            />
          ) : (
            <>
              <div className="queue-head">
                <span className="count">{files.length}</span>
                <span className="size">个文件 · {humanBytes(scan?.total_bytes ?? 0)}</span>
                <span className="spacer" />
                <button className="link-btn" onClick={pickFiles}>
                  添加文件
                </button>
                <button className="link-btn" onClick={pickFolder} disabled={batch.phase === 'running'}>
                  添加文件夹
                </button>
                <button className="link-btn danger" onClick={clearAll} disabled={batch.phase === 'running'}>
                  清空
                </button>
              </div>

              {scan?.skipped?.length > 0 && (
                <div className="skip-note">
                  已忽略：
                  {scan.skipped.map((s) => `${s.reason} ${s.count}`).join(' · ')}
                </div>
              )}

              <FileQueue
                files={files}
                status={batch.status}
                target={target}
                collapsing={collapsing}
                onRemove={batch.phase === 'running' ? null : removeFile}
                onReveal={reveal}
              />

              <div className="config card">
                {commonTargets.length > 0 ? (
                  <>
                    <div className="card-title">
                      转换目标
                      {runnable.length !== files.length && (
                        <span className="muted">
                          {runnable.length}/{files.length} 个可转换
                        </span>
                      )}
                    </div>
                    <TargetPicker
                      targets={commonTargets}
                      byId={byId}
                      value={target}
                      onChange={setTarget}
                      files={files}
                      disabled={batch.phase === 'running'}
                    />                    {activeOptions.length > 0 && (
                      <OptionPanel
                        defs={activeOptions}
                        values={options}
                        onChange={(key, value) =>
                          setSettings((s) => ({
                            ...s,
                            options: { ...s.options, [key]: value },
                          }))
                        }
                        disabled={batch.phase === 'running'}
                      />
                    )}
                  </>
                ) : (
                  <div className="empty-note">
                    所选文件没有共同的转换目标。
                    <br />
                    请分批处理，或移除部分文件。
                  </div>
                )}
              </div>

              {batch.fatal && <div className="fatal-note">{batch.fatal}</div>}
            </>
          )}
        </div>

        {showQueue && (
          <ConvertBar
            phase={batch.phase}
            tick={batch.tick}
            summary={batch.summary}
            canRun={canRun}
            target={labelFor(target, byId)}
            fileCount={runnable.length}
            onRun={run}
            onCancel={batch.cancel}
            onOpenOutput={openOutput}
            onReset={() => batch.reset()}
          />
        )}
      </div>

      <DropOverlay active={over} pos={pos} />

      {sheetOpen && (
        <EngineSheet
          report={engines}
          probing={probing}
          onRefresh={refreshEngines}
          onClose={() => setSheetOpen(false)}
        />
      )}
    </div>
  );
}

/* ---------- 局部工具 ---------- */

function intersectTargets(files) {
  const usable = files.filter((f) => !f.unsupported);
  if (!usable.length) return [];
  return usable
    .map((f) => f.targets)
    .reduce((acc, cur) => acc.filter((t) => cur.includes(t)));
}

function humanBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB'];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}
