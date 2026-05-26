import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import { Icons } from "../../icons";
import type { DedupeSummary, ExtensionRepairSummary, ScanSummary, SettingsState, ThumbnailProgress } from "../../types";
import {
  formatCount,
  formatDedupeSummary,
  formatErrorMessage,
  formatRepairSummary,
  formatScanSummary,
  setPageBackground,
  uniquePaths,
} from "../../utils";

type StatusTone = "" | "ok" | "error";
type TaskName = "scan" | "dedupe" | "repair";

interface StatusState {
  message: string;
  tone: StatusTone;
}

export function SettingsView() {
  const [paths, setPaths] = useState<string[]>([]);
  const [savedPaths, setSavedPaths] = useState<string[]>([]);
  const [imageCount, setImageCount] = useState(0);
  const [dbPath, setDbPath] = useState("");
  const [xaiKey, setXaiKey] = useState("");
  const [savedXaiKey, setSavedXaiKey] = useState("");
  const [generatedDir, setGeneratedDir] = useState("");
  const [savedGeneratedDir, setSavedGeneratedDir] = useState("");
  const [thumbnailEnabled, setThumbnailEnabled] = useState(false);
  const [savedThumbnailEnabled, setSavedThumbnailEnabled] = useState(false);
  const [thumbnailDir, setThumbnailDir] = useState("");
  const [savedThumbnailDir, setSavedThumbnailDir] = useState("");
  const [hasGap, setHasGap] = useState(true);
  const [savedHasGap, setSavedHasGap] = useState(true);
  const [theme, setTheme] = useState<"black" | "white">("white");
  const [savedTheme, setSavedTheme] = useState<"black" | "white">("white");
  const [minColumnWidth, setMinColumnWidth] = useState(280);
  const [savedMinColumnWidth, setSavedMinColumnWidth] = useState(280);
  const [status, setStatus] = useState<StatusState>({ message: "", tone: "" });
  const [thumbnailProgress, setThumbnailProgress] = useState<ThumbnailProgress | null>(null);
  const [runningTask, setRunningTask] = useState<TaskName | null>(null);
  const [pickingGeneratedDir, setPickingGeneratedDir] = useState(false);
  const [pickingThumbnailDir, setPickingThumbnailDir] = useState(false);
  const [addingPaths, setAddingPaths] = useState(false);

  useEffect(() => {
    setPageBackground("#f7f7f7");
    loadSettings().catch((error) => {
      setStatus({ message: formatErrorMessage(error, "加载失败"), tone: "error" });
    });
  }, []);

  useEffect(() => {
    async function pollProgress() {
      try {
        const progress = await invoke<ThumbnailProgress>("get_thumbnail_progress");
        setThumbnailProgress(progress);
      } catch {
        // Keep polling lightweight; transient backend errors should not break settings.
      }
    }

    pollProgress();
    const timer = window.setInterval(pollProgress, 1000);
    return () => {
      window.clearInterval(timer);
    };
  }, []);

  const dirty = useMemo(() => {
    return (
      !pathsEqual(paths, savedPaths) ||
      hasGap !== savedHasGap ||
      theme !== savedTheme ||
      minColumnWidth !== savedMinColumnWidth ||
      xaiKey !== savedXaiKey ||
      generatedDir !== savedGeneratedDir ||
      thumbnailEnabled !== savedThumbnailEnabled ||
      thumbnailDir !== savedThumbnailDir
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
    thumbnailEnabled,
    savedThumbnailEnabled,
    thumbnailDir,
    savedThumbnailDir,
  ]);

  const thumbnailTaskRunning = thumbnailProgress?.running === true;
  const taskDisabled = dirty || runningTask !== null || thumbnailTaskRunning;

  async function loadSettings() {
    const settings = await invoke<SettingsState>("get_settings");
    const loadedPaths = uniquePaths(settings.paths);
    setPaths(loadedPaths);
    setSavedPaths(loadedPaths);
    setImageCount(settings.imageCount);
    setDbPath(settings.dbPath);
    setXaiKey(settings.xaiKey || "");
    setSavedXaiKey(settings.xaiKey || "");
    setGeneratedDir(settings.generatedContentDir || "");
    setSavedGeneratedDir(settings.generatedContentDir || "");
    setThumbnailEnabled(Boolean(settings.thumbnailEnabled));
    setSavedThumbnailEnabled(Boolean(settings.thumbnailEnabled));
    setThumbnailDir(settings.thumbnailDir || "");
    setSavedThumbnailDir(settings.thumbnailDir || "");
    setHasGap(settings.galleryHasGap);
    setSavedHasGap(settings.galleryHasGap);
    setTheme(settings.galleryTheme === "black" ? "black" : "white");
    setSavedTheme(settings.galleryTheme === "black" ? "black" : "white");
    setMinColumnWidth(settings.minColumnWidth || 280);
    setSavedMinColumnWidth(settings.minColumnWidth || 280);
    setStatus({ message: "", tone: "" });
  }

  async function saveAll(tone: StatusTone = "ok") {
    const shouldStartThumbnailScan = thumbnailEnabled;
    if (shouldStartThumbnailScan) {
      const confirmed = window.confirm(
        "缩略图模式会扫描素材目录，并为图片生成持久化缓存。该过程可能耗时并占用额外磁盘空间；视频不会生成缩略图。确认保存并开始扫描吗？",
      );
      if (!confirmed) {
        setStatus({ message: "已取消保存", tone: "" });
        return;
      }
    }

    await invoke("save_gallery_preferences", {
      mode: hasGap ? theme : "none",
      hasGap,
      theme,
      minColumnWidth,
    });
    await invoke("save_xai_settings", {
      xaiKey,
      generatedContentDir: generatedDir,
    });
    await invoke("save_thumbnail_settings", {
      thumbnailEnabled,
      thumbnailDir,
    });
    const storedPaths = await invoke<string[]>("save_source_paths", { paths });
    const normalizedPaths = uniquePaths(storedPaths);
    setPaths(normalizedPaths);
    setSavedPaths(normalizedPaths);
    setSavedHasGap(hasGap);
    setSavedTheme(theme);
    setSavedMinColumnWidth(minColumnWidth);
    setSavedXaiKey(xaiKey);
    setSavedGeneratedDir(generatedDir);
    setSavedThumbnailEnabled(thumbnailEnabled);
    setSavedThumbnailDir(thumbnailDir);
    if (shouldStartThumbnailScan) {
      await invoke("start_thumbnail_scan", { paths: normalizedPaths });
      setThumbnailProgress({
        running: true,
        stage: "scanning",
        processed: 0,
        total: 0,
        generated: 0,
        skipped: 0,
        message: "扫描素材中...",
        error: "",
      });
      setStatus({ message: "", tone: "" });
    } else {
      setStatus({ message: "已保存", tone });
    }
  }

  async function handleSave() {
    setStatus({ message: "保存中...", tone: "" });
    try {
      await saveAll();
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, "保存失败"), tone: "error" });
    }
  }

  async function handleAddPaths() {
    setAddingPaths(true);
    try {
      const selectedPaths = await invoke<string[]>("pick_source_folders");
      setPaths((current) => uniquePaths([...current, ...selectedPaths]));
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, "添加目录失败"), tone: "error" });
    } finally {
      setAddingPaths(false);
    }
  }

  async function handlePickGeneratedDir() {
    setPickingGeneratedDir(true);
    try {
      const selectedPath = await invoke<string | null>("pick_generated_content_folder");
      if (selectedPath) setGeneratedDir(selectedPath);
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, "选择目录失败"), tone: "error" });
    } finally {
      setPickingGeneratedDir(false);
    }
  }

  async function handlePickThumbnailDir() {
    setPickingThumbnailDir(true);
    try {
      const selectedPath = await invoke<string | null>("pick_thumbnail_folder");
      if (selectedPath) setThumbnailDir(selectedPath);
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, "选择目录失败"), tone: "error" });
    } finally {
      setPickingThumbnailDir(false);
    }
  }

  async function runTask<T>(
    taskName: TaskName,
    pendingMessage: string,
    fallback: string,
    command: () => Promise<T>,
    summarize: (summary: T) => string,
    imageCountFromSummary: (summary: T) => number,
  ) {
    if (dirty) {
      setStatus({ message: "请先保存", tone: "error" });
      return;
    }

    setRunningTask(taskName);
    setStatus({ message: pendingMessage, tone: "" });
    try {
      const summary = await command();
      setImageCount(imageCountFromSummary(summary));
      setStatus({ message: summarize(summary), tone: "ok" });
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, fallback), tone: "error" });
    } finally {
      setRunningTask(null);
    }
  }

  async function handleScan() {
    await runTask<ScanSummary>(
      "scan",
      "扫描中...",
      "扫描失败",
      () => invoke("scan_library", { paths: savedPaths }),
      formatScanSummary,
      (summary) => summary.total,
    );
  }

  async function handleDedupe() {
    if (dirty) {
      setStatus({ message: "请先保存", tone: "error" });
      return;
    }

    setRunningTask("dedupe");
    setStatus({ message: "选择目录...", tone: "" });
    try {
      const destinationPath = await invoke<string | null>("pick_duplicate_folder");
      if (!destinationPath) {
        setStatus({ message: "", tone: "" });
        return;
      }
      setStatus({ message: "检测中...", tone: "" });
      const summary = await invoke<DedupeSummary>("deduplicate_resources", {
        paths: savedPaths,
        destinationPath,
      });
      setImageCount(summary.total);
      setStatus({ message: formatDedupeSummary(summary), tone: "ok" });
    } catch (error) {
      setStatus({ message: formatErrorMessage(error, "检测失败"), tone: "error" });
    } finally {
      setRunningTask(null);
    }
  }

  async function handleRepair() {
    await runTask<ExtensionRepairSummary>(
      "repair",
      "修复中...",
      "修复失败",
      () => invoke("repair_image_extensions", { paths: savedPaths }),
      formatRepairSummary,
      (summary) => summary.total,
    );
  }

  return (
    <main className="settings-shell">
      <section className="settings-panel">
        <div className="settings-header">
          <h1>设置</h1>
          <button className="secondary-button icon-button" type="button" onClick={() => invoke("open_app_window", { label: "gallery" })}>
            <Icons.ArrowTopRight />
            <span>打开瀑布流</span>
          </button>
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
                        onClick={() => setPaths((current) => current.filter((_, itemIndex) => itemIndex !== index))}
                      >
                        <Icons.XMark />
                      </button>
                    </div>
                  ))
                )}
              </div>
              <button className="secondary-button icon-button add-path-button" type="button" disabled={addingPaths} onClick={handleAddPaths}>
                <Icons.Plus />
                <span>添加文件夹</span>
              </button>
            </div>
          </div>

          <div className="field field-output">
            <span className="field-head">
              <span className="field-label">索引数据 <span className="field-meta">{formatCount(imageCount)}</span></span>
            </span>
            <output className="field-output-value field-output-break">{dbPath}</output>
          </div>

          <div className="field">
            <span className="field-head">
              <span className="field-label">xAI API Key</span>
            </span>
            <input className="text-input" id="xai-key" type="password" autoComplete="off" value={xaiKey} onChange={(event) => setXaiKey(event.currentTarget.value)} />
          </div>

          <div className="field">
            <span className="field-head">
              <span className="field-label">生成内容保存位置</span>
            </span>
            <div className="single-path-row">
              <button className="secondary-button icon-only-button" type="button" aria-label="选择目录" disabled={pickingGeneratedDir} onClick={handlePickGeneratedDir}>
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
              <p className="field-help">
                修复扩展名会根据图片真实格式重命名错误扩展名文件；检测重复会按文件大小和内容哈希查找重复资源，并在确认目录后移动重复文件。两个操作都会直接处理素材文件，请先确认素材目录已保存。
              </p>
              <div className="maintenance-actions">
                <button className={`secondary-button task-button${runningTask === "repair" ? " is-running" : ""}`} type="button" disabled={taskDisabled} onClick={handleRepair}>
                  <Icons.ArrowPath />
                  <span>修复扩展名</span>
                </button>
                <button className={`secondary-button task-button${runningTask === "dedupe" ? " is-running" : ""}`} type="button" disabled={taskDisabled} onClick={handleDedupe}>
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
                <input type="radio" name="gap-mode" value="none" checked={!hasGap} onChange={() => setHasGap(false)} />
                <span>无间距</span>
              </label>
              <label className="choice-pill">
                <input type="radio" name="gap-mode" value="gap" checked={hasGap} onChange={() => setHasGap(true)} />
                <span>有间距</span>
              </label>
              <label className="choice-pill">
                <input type="radio" name="gallery-theme" value="black" checked={theme === "black"} onChange={() => setTheme("black")} />
                <span>黑色背景</span>
              </label>
              <label className="choice-pill">
                <input type="radio" name="gallery-theme" value="white" checked={theme === "white"} onChange={() => setTheme("white")} />
                <span>白色背景</span>
              </label>
            </div>
          </div>

          <div className="field">
            <span className="field-head">
              <span className="field-label">图库性能</span>
            </span>
            <div className="field-body">
              <div className="choice-group theme-choice-group" role="group" aria-label="图库性能">
                <label className="choice-pill">
                  <input type="radio" name="thumbnail-mode" value="original" checked={!thumbnailEnabled} onChange={() => setThumbnailEnabled(false)} />
                  <span>原图模式</span>
                </label>
                <label className="choice-pill">
                  <input type="radio" name="thumbnail-mode" value="thumbnail" checked={thumbnailEnabled} onChange={() => setThumbnailEnabled(true)} />
                  <span>缩略图模式</span>
                </label>
              </div>
              <p className="field-help">
                原图模式不生成额外文件，适合小图库或不希望占用额外磁盘空间。缩略图模式只为图片生成可删除缓存，保存时会先确认并启动后台扫描；扫描生成期间会消耗一些 CPU 和磁盘空间，预览和编辑仍使用原图，视频保持现有加载方式。
              </p>
              {thumbnailProgress && (thumbnailProgress.running || thumbnailProgress.message) ? (
                <div className="thumbnail-progress" data-running={thumbnailProgress.running ? "true" : "false"}>
                  <span>{thumbnailProgress.message}</span>
                  {thumbnailProgress.total > 0 ? (
                    <span>{thumbnailProgress.processed} / {thumbnailProgress.total}</span>
                  ) : null}
                </div>
              ) : null}
              <div className="single-path-row">
                <button className="secondary-button icon-only-button" type="button" aria-label="选择缩略图目录" disabled={pickingThumbnailDir} onClick={handlePickThumbnailDir}>
                  <Icons.Folder />
                </button>
                <output className="single-path-value">{thumbnailDir}</output>
              </div>
            </div>
          </div>

          <div className="field">
            <span className="field-head">
              <span className="field-label">瀑布流列宽 <span className="field-meta">{minColumnWidth}px</span></span>
            </span>
            <input className="range-input" type="range" min="100" max="600" step="1" value={minColumnWidth} onChange={(event) => setMinColumnWidth(Number(event.currentTarget.value) || 280)} />
          </div>
        </div>

        <div className="settings-actions">
          <p className="status-line" data-tone={status.tone}>{status.message}</p>
          <div className="settings-action-buttons">
            <button className={`secondary-button task-button${runningTask === "scan" ? " is-running" : ""}`} type="button" disabled={taskDisabled} onClick={handleScan}>
              <Icons.ArrowPath />
              <span>扫描</span>
            </button>
            <button className="primary-button" type="button" disabled={!dirty || thumbnailTaskRunning} onClick={handleSave}>保存</button>
          </div>
        </div>
      </section>
    </main>
  );
}

function pathsEqual(left: string[], right: string[]) {
  return left.length === right.length && left.every((path, index) => path === right[index]);
}
