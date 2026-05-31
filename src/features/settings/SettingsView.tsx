import { invoke } from '@tauri-apps/api/core';
import { useEffect, useMemo, useState } from 'react';
import { Icons } from '../../icons';
import type {
  DedupeSummary,
  ExtensionRepairSummary,
  ScanSummary,
  SettingsState,
  WindowsCloseBehavior,
} from '../../types';
import {
  formatCount,
  formatDedupeSummary,
  formatErrorMessage,
  formatRepairSummary,
  formatScanSummary,
  setPageBackground,
  storeGalleryTheme,
  uniquePaths,
} from '../../utils';

type StatusTone = '' | 'ok' | 'error';
type TaskName = 'scan' | 'dedupe' | 'repair';

interface StatusState {
  message: string;
  tone: StatusTone;
}

function normalizeWindowsCloseBehavior(value: string): WindowsCloseBehavior {
  if (value === 'exit' || value === 'tray' || value === 'ask') return value;
  return 'ask';
}

export function SettingsView() {
  const [platform, setPlatform] = useState('');
  const [paths, setPaths] = useState<string[]>([]);
  const [savedPaths, setSavedPaths] = useState<string[]>([]);
  const [imageCount, setImageCount] = useState(0);
  const [dbPath, setDbPath] = useState('');
  const [xaiKey, setXaiKey] = useState('');
  const [savedXaiKey, setSavedXaiKey] = useState('');
  const [generatedDir, setGeneratedDir] = useState('');
  const [savedGeneratedDir, setSavedGeneratedDir] = useState('');
  const [hasGap, setHasGap] = useState(true);
  const [savedHasGap, setSavedHasGap] = useState(true);
  const [theme, setTheme] = useState<'black' | 'white'>('white');
  const [savedTheme, setSavedTheme] = useState<'black' | 'white'>('white');
  const [minColumnWidth, setMinColumnWidth] = useState(280);
  const [savedMinColumnWidth, setSavedMinColumnWidth] = useState(280);
  const [windowsCloseBehavior, setWindowsCloseBehavior] =
    useState<WindowsCloseBehavior>('ask');
  const [savedWindowsCloseBehavior, setSavedWindowsCloseBehavior] =
    useState<WindowsCloseBehavior>('ask');
  const [status, setStatus] = useState<StatusState>({ message: '', tone: '' });
  const [runningTask, setRunningTask] = useState<TaskName | null>(null);
  const [pickingGeneratedDir, setPickingGeneratedDir] = useState(false);
  const [addingPaths, setAddingPaths] = useState(false);
  const isWindows = platform === 'windows';

  useEffect(() => {
    setPageBackground('#f7f7f7');
    loadSettings().catch((error) => {
      setStatus({ message: formatErrorMessage(error, '加载失败'), tone: 'error' });
    });
  }, []);

  const dirty = useMemo(() => {
    return (
      !pathsEqual(paths, savedPaths) ||
      hasGap !== savedHasGap ||
      theme !== savedTheme ||
      minColumnWidth !== savedMinColumnWidth ||
      xaiKey !== savedXaiKey ||
      generatedDir !== savedGeneratedDir ||
      (isWindows && windowsCloseBehavior !== savedWindowsCloseBehavior)
    );
  }, [
    paths,
    savedPaths,
    hasGap,
    savedHasGap,
    theme,
    savedTheme,
    minColumnWidth,
    savedMinColumnWidth,
    xaiKey,
    savedXaiKey,
    generatedDir,
    savedGeneratedDir,
    isWindows,
    windowsCloseBehavior,
    savedWindowsCloseBehavior,
  ]);

  const taskDisabled = dirty || runningTask !== null;

  async function loadSettings() {
    const settings = await invoke<SettingsState>('get_settings');
    const loadedPaths = uniquePaths(settings.paths);
    const closeBehavior = normalizeWindowsCloseBehavior(settings.windowsCloseBehavior);
    setPlatform(settings.platform || '');
    setPaths(loadedPaths);
    setSavedPaths(loadedPaths);
    setImageCount(settings.imageCount);
    setDbPath(settings.dbPath);
    setXaiKey(settings.xaiKey || '');
    setSavedXaiKey(settings.xaiKey || '');
    setGeneratedDir(settings.generatedContentDir || '');
    setSavedGeneratedDir(settings.generatedContentDir || '');
    setHasGap(settings.galleryHasGap);
    setSavedHasGap(settings.galleryHasGap);
    setTheme(settings.galleryTheme === 'black' ? 'black' : 'white');
    setSavedTheme(settings.galleryTheme === 'black' ? 'black' : 'white');
    setMinColumnWidth(settings.minColumnWidth || 280);
    setSavedMinColumnWidth(settings.minColumnWidth || 280);
    setWindowsCloseBehavior(closeBehavior);
    setSavedWindowsCloseBehavior(closeBehavior);
    setStatus({ message: '', tone: '' });
  }

  async function saveAll(tone: StatusTone = 'ok') {
    await invoke('save_gallery_preferences', {
      mode: hasGap ? theme : 'none',
      hasGap,
      theme,
      minColumnWidth,
    });
    storeGalleryTheme(theme);
    await invoke('save_xai_settings', {
      xaiKey,
      generatedContentDir: generatedDir,
    });
    await invoke('save_thumbnail_settings', {
      thumbnailEnabled: false,
      thumbnailDir: '',
    });
    const storedWindowsCloseBehavior = isWindows
      ? await invoke<WindowsCloseBehavior>('save_windows_close_behavior', {
          closeBehavior: windowsCloseBehavior,
        })
      : windowsCloseBehavior;
    const storedPaths = await invoke<string[]>('save_source_paths', { paths });
    const normalizedPaths = uniquePaths(storedPaths);
    setPaths(normalizedPaths);
    setSavedPaths(normalizedPaths);
    setSavedHasGap(hasGap);
    setSavedTheme(theme);
    setSavedMinColumnWidth(minColumnWidth);
    setSavedXaiKey(xaiKey);
    setSavedGeneratedDir(generatedDir);
    setWindowsCloseBehavior(storedWindowsCloseBehavior);
    setSavedWindowsCloseBehavior(storedWindowsCloseBehavior);
    setStatus({ message: '扫描中...', tone: '' });
    const scanSummary = await invoke<ScanSummary>('scan_library', { paths: normalizedPaths });
    setImageCount(scanSummary.total);
    setStatus({ message: formatScanSummary(scanSummary), tone });
  }

  async function handleSave() {
    setStatus({ message: '保存中...', tone: '' });
    try {
      await saveAll();
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, '保存失败'), tone: 'error' });
    }
  }

  async function handleAddPaths() {
    setAddingPaths(true);
    try {
      const selectedPaths = await invoke<string[]>('pick_source_folders');
      setPaths((current) => uniquePaths([...current, ...selectedPaths]));
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, '添加目录失败'), tone: 'error' });
    } finally {
      setAddingPaths(false);
    }
  }

  async function handlePickGeneratedDir() {
    setPickingGeneratedDir(true);
    try {
      const selectedPath = await invoke<string | null>('pick_generated_content_folder');
      if (selectedPath) setGeneratedDir(selectedPath);
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, '选择目录失败'), tone: 'error' });
    } finally {
      setPickingGeneratedDir(false);
    }
  }

  async function runTask<T>(
    taskName: TaskName,
    pendingMessage: string,
    fallback: string,
    command: () => Promise<T>,
    summarize: (summary: T) => string,
    imageCountFromSummary: (summary: T) => number
  ) {
    if (dirty) {
      setStatus({ message: '请先保存', tone: 'error' });
      return;
    }

    setRunningTask(taskName);
    setStatus({ message: pendingMessage, tone: '' });
    try {
      const summary = await command();
      setImageCount(imageCountFromSummary(summary));
      setStatus({ message: summarize(summary), tone: 'ok' });
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, fallback), tone: 'error' });
    } finally {
      setRunningTask(null);
    }
  }

  async function handleScan() {
    await runTask<ScanSummary>(
      'scan',
      '扫描中...',
      '扫描失败',
      () => invoke('scan_library', { paths: savedPaths }),
      formatScanSummary,
      (summary) => summary.total
    );
  }

  async function handleDedupe() {
    if (dirty) {
      setStatus({ message: '请先保存', tone: 'error' });
      return;
    }

    setRunningTask('dedupe');
    setStatus({ message: '选择目录...', tone: '' });
    try {
      const destinationPath = await invoke<string | null>('pick_duplicate_folder');
      if (!destinationPath) {
        setStatus({ message: '', tone: '' });
        return;
      }
      setStatus({ message: '检测中...', tone: '' });
      const summary = await invoke<DedupeSummary>('deduplicate_resources', {
        paths: savedPaths,
        destinationPath,
      });
      setImageCount(summary.total);
      setStatus({ message: formatDedupeSummary(summary), tone: 'ok' });
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, '检测失败'), tone: 'error' });
    } finally {
      setRunningTask(null);
    }
  }

  async function handleRepair() {
    await runTask<ExtensionRepairSummary>(
      'repair',
      '修复中...',
      '修复失败',
      () => invoke('repair_image_extensions', { paths: savedPaths }),
      formatRepairSummary,
      (summary) => summary.total
    );
  }

  async function handleOpenGallery() {
    try {
      await invoke('open_gallery_from_settings');
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, '打开失败'), tone: 'error' });
    }
  }

  async function handleOpenCarousel() {
    try {
      await invoke('open_carousel_from_settings');
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, '打开失败'), tone: 'error' });
    }
  }

  return (
    <main className="settings-shell">
      <section className="settings-panel">
        <div className="settings-header">
          <h1>设置</h1>
          <div className="settings-header-actions">
            <button
              className="secondary-button icon-button"
              type="button"
              onClick={handleOpenGallery}
            >
              <Icons.ArrowTopRight />
              <span>打开瀑布流</span>
            </button>
            <button
              className="secondary-button icon-button"
              type="button"
              onClick={handleOpenCarousel}
            >
              <Icons.ArrowTopRight />
              <span>打开走马灯</span>
            </button>
          </div>
        </div>

        <div className="settings-form">
          <div className="field">
            <span className="field-head">
              <span className="field-label">素材文件夹</span>
            </span>
            <div className="field-body">
              <div className="path-list">
                {paths.length === 0 ? (
                  <div className="path-row path-row-empty">未添加</div>
                ) : (
                  paths.map((path, index) => (
                    <div className="path-row" key={path}>
                      <span className="path-value">{path}</span>
                      <button
                        className="path-remove-button"
                        type="button"
                        aria-label="移除目录"
                        onClick={() =>
                          setPaths((current) =>
                            current.filter((_, itemIndex) => itemIndex !== index)
                          )
                        }
                      >
                        <Icons.XMark />
                      </button>
                    </div>
                  ))
                )}
              </div>
              <button
                className="secondary-button icon-button add-path-button"
                type="button"
                disabled={addingPaths}
                onClick={handleAddPaths}
              >
                <Icons.Plus />
                <span>添加文件夹</span>
              </button>
            </div>
          </div>

          <div className="field field-output">
            <span className="field-head">
              <span className="field-label">
                索引数据 <span className="field-meta">{formatCount(imageCount)}</span>
              </span>
            </span>
            <output className="field-output-value field-output-break">{dbPath}</output>
          </div>

          <div className="field">
            <span className="field-head">
              <span className="field-label">xAI API Key</span>
            </span>
            <input
              className="text-input"
              id="xai-key"
              type="password"
              autoComplete="off"
              value={xaiKey}
              onChange={(event) => setXaiKey(event.currentTarget.value)}
            />
          </div>

          <div className="field">
            <span className="field-head">
              <span className="field-label">生成内容保存位置</span>
            </span>
            <div className="single-path-row">
              <button
                className="secondary-button icon-only-button"
                type="button"
                aria-label="选择目录"
                disabled={pickingGeneratedDir}
                onClick={handlePickGeneratedDir}
              >
                <Icons.Folder />
              </button>
              <output className="single-path-value">{generatedDir}</output>
            </div>
          </div>

          <div className="field">
            <span className="field-head">
              <span className="field-label">资源维护</span>
            </span>
            <div className="field-body">
              <div className="maintenance-actions">
                <button
                  className={`secondary-button task-button${runningTask === 'repair' ? ' is-running' : ''}`}
                  type="button"
                  disabled={taskDisabled}
                  onClick={handleRepair}
                >
                  <Icons.ArrowPath />
                  <span>修复扩展名</span>
                </button>
                <button
                  className={`secondary-button task-button${runningTask === 'dedupe' ? ' is-running' : ''}`}
                  type="button"
                  disabled={taskDisabled}
                  onClick={handleDedupe}
                >
                  <Icons.CheckBadge />
                  <span>检测重复</span>
                </button>
              </div>
            </div>
          </div>

          <div className="field">
            <span className="field-head">
              <span className="field-label">瀑布流外观</span>
            </span>
            <div className="choice-group theme-choice-group" role="group" aria-label="主题">
              <label className="choice-pill">
                <input
                  type="radio"
                  name="gap-mode"
                  value="none"
                  checked={!hasGap}
                  onChange={() => setHasGap(false)}
                />
                <span>无间距</span>
              </label>
              <label className="choice-pill">
                <input
                  type="radio"
                  name="gap-mode"
                  value="gap"
                  checked={hasGap}
                  onChange={() => setHasGap(true)}
                />
                <span>有间距</span>
              </label>
              <label className="choice-pill">
                <input
                  type="radio"
                  name="gallery-theme"
                  value="black"
                  checked={theme === 'black'}
                  onChange={() => setTheme('black')}
                />
                <span>黑色背景</span>
              </label>
              <label className="choice-pill">
                <input
                  type="radio"
                  name="gallery-theme"
                  value="white"
                  checked={theme === 'white'}
                  onChange={() => setTheme('white')}
                />
                <span>白色背景</span>
              </label>
            </div>
          </div>

          <div className="field">
            <span className="field-head">
              <span className="field-label">
                瀑布流列宽 <span className="field-meta">{minColumnWidth}px</span>
              </span>
            </span>
            <input
              className="range-input"
              type="range"
              min="100"
              max="600"
              step="1"
              value={minColumnWidth}
              onChange={(event) => setMinColumnWidth(Number(event.currentTarget.value) || 280)}
            />
          </div>

          {isWindows && (
            <div className="field">
              <span className="field-head">
                <span className="field-label">关闭窗口</span>
              </span>
              <div className="choice-group" role="radiogroup" aria-label="关闭窗口">
                <label className="choice-pill">
                  <input
                    type="radio"
                    name="windows-close-behavior"
                    value="ask"
                    checked={windowsCloseBehavior === 'ask'}
                    onChange={() => setWindowsCloseBehavior('ask')}
                  />
                  <span>每次询问</span>
                </label>
                <label className="choice-pill">
                  <input
                    type="radio"
                    name="windows-close-behavior"
                    value="exit"
                    checked={windowsCloseBehavior === 'exit'}
                    onChange={() => setWindowsCloseBehavior('exit')}
                  />
                  <span>退出应用</span>
                </label>
                <label className="choice-pill">
                  <input
                    type="radio"
                    name="windows-close-behavior"
                    value="tray"
                    checked={windowsCloseBehavior === 'tray'}
                    onChange={() => setWindowsCloseBehavior('tray')}
                  />
                  <span>保留托盘</span>
                </label>
              </div>
            </div>
          )}
        </div>

        <div className="settings-actions">
          <p className="status-line" data-tone={status.tone}>
            {status.message}
          </p>
          <div className="settings-action-buttons">
            <button
              className={`secondary-button task-button${runningTask === 'scan' ? ' is-running' : ''}`}
              type="button"
              disabled={taskDisabled}
              onClick={handleScan}
            >
              <Icons.ArrowPath />
              <span>扫描</span>
            </button>
            <button className="primary-button" type="button" disabled={!dirty} onClick={handleSave}>
              保存
            </button>
          </div>
        </div>
      </section>
    </main>
  );
}

function pathsEqual(left: string[], right: string[]) {
  return left.length === right.length && left.every((path, index) => path === right[index]);
}
