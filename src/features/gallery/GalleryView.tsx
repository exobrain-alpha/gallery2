import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { EditorDrawer, type EditorDrawerHandle } from "../editor/EditorDrawer";
import { Icons } from "../../icons";
import type {
  GalleryPreferences,
  ImageCursor,
  ImagePage,
  ImageRecord,
  PickedImage,
  XaiEditResult,
} from "../../types";
import { classNames, logError, setPageBackground, storeGalleryTheme, storedGalleryTheme } from "../../utils";
import { buildLayout } from "./layout";
import { MediaPlaceholder, PreviewImage, TileImage } from "./MediaTile";

const PAGE_SIZE = 50;
const OVERSCAN = 1200;
const MAX_REFERENCE_SELECTION = 3;
const MAX_PLAYING_TILE_VIDEOS = 10;
const TILE_VIDEO_PREVIEW_TIME = 0.08;

interface ViewportState {
  scrollY: number;
  width: number;
  height: number;
}

type PreviewState =
  | { type: "image"; src: string; path?: string }
  | { type: "video"; src: string; width: number; height: number }
  | null;

interface ContextMenuState {
  left: number;
  top: number;
  record: ImageRecord;
}

export function GalleryView() {
  const [preferences, setPreferences] = useState<GalleryPreferences | null>(null);
  const initialTheme = useMemo(() => storedGalleryTheme(), []);
  const [records, setRecords] = useState<ImageRecord[]>([]);
  const [done, setDone] = useState(false);
  const [viewport, setViewport] = useState<ViewportState>(() => ({
    scrollY: window.scrollY,
    width: window.innerWidth,
    height: window.innerHeight,
  }));
  const [preview, setPreview] = useState<PreviewState>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [selectedReferenceRecords, setSelectedReferenceRecords] = useState<ImageRecord[]>([]);
  const sentinelRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<EditorDrawerHandle | null>(null);
  const loadingRef = useRef(false);
  const doneRef = useRef(false);
  const recordsRef = useRef<ImageRecord[]>([]);
  const cursorRef = useRef<ImageCursor | null>(null);
  const selectedReferenceRecordsRef = useRef<ImageRecord[]>([]);
  const playingVideosRef = useRef<Map<string, HTMLVideoElement>>(new Map());

  useEffect(() => {
    setPageBackground(initialTheme === "black" ? "#1a1b1e" : "#ffffff");
    invoke<GalleryPreferences>("get_gallery_preferences")
      .then((loadedPreferences) => {
        setPreferences(loadedPreferences);
        storeGalleryTheme(loadedPreferences.theme);
        setPageBackground(loadedPreferences.theme === "black" ? "#1a1b1e" : "#ffffff");
      })
      .catch((error) => logError(error, "Failed to load gallery preferences"));
  }, [initialTheme]);

  useEffect(() => {
    let frame = 0;
    const updateViewport = () => {
      if (frame) return;
      frame = window.requestAnimationFrame(() => {
        frame = 0;
        setViewport({
          scrollY: window.scrollY,
          width: window.innerWidth,
          height: window.innerHeight,
        });
      });
    };

    window.addEventListener("scroll", updateViewport, { passive: true });
    window.addEventListener("resize", updateViewport, { passive: true });
    return () => {
      if (frame) window.cancelAnimationFrame(frame);
      window.removeEventListener("scroll", updateViewport);
      window.removeEventListener("resize", updateViewport);
    };
  }, []);

  useEffect(() => {
    document.body.classList.toggle("previewing", preview !== null);
    return () => document.body.classList.remove("previewing");
  }, [preview]);

  useEffect(() => {
    selectedReferenceRecordsRef.current = selectedReferenceRecords;
  }, [selectedReferenceRecords]);

  useEffect(() => {
    const handleClick = (event: MouseEvent) => {
      if (contextMenu && !(event.target as Element | null)?.closest("#context-menu")) {
        setContextMenu(null);
      }
    };
    const handleContextMenu = (event: MouseEvent) => {
      if (!(event.target as Element | null)?.closest(".image-tile")) setContextMenu(null);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (contextMenu) {
        setContextMenu(null);
        return;
      }
      if (preview) {
        setPreview(null);
        return;
      }
      if (editorRef.current?.isOpen()) {
        editorRef.current.close();
      }
    };

    window.addEventListener("click", handleClick);
    window.addEventListener("contextmenu", handleContextMenu);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("click", handleClick);
      window.removeEventListener("contextmenu", handleContextMenu);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [contextMenu, preview]);

  useEffect(() => {
    loadMore().catch((error) => logError(error, "Failed to load gallery page"));
  }, []);

  useEffect(() => {
    const reloadEmptyGallery = () => {
      if (recordsRef.current.length > 0) return;
      reloadFirstPage().catch((error) => logError(error, "Failed to reload gallery page"));
    };

    window.addEventListener("gallery:reload", reloadEmptyGallery);
    const timer = window.setInterval(reloadEmptyGallery, 2000);
    return () => {
      window.removeEventListener("gallery:reload", reloadEmptyGallery);
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    const sentinel = sentinelRef.current;
    if (!sentinel) return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        loadMore().catch((error) => logError(error, "Failed to load gallery page"));
      }
    }, { rootMargin: "1200px" });
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [preferences]);

  useEffect(() => {
    return () => {
      for (const video of playingVideosRef.current.values()) {
        pauseVideoElement(video);
      }
      playingVideosRef.current.clear();
    };
  }, []);

  const gapSize = preferences?.hasGap ? 12 : 0;
  const minColumnWidth = Math.min(600, Math.max(100, preferences?.minColumnWidth || 280));
  const layoutItems = useMemo(
    () => buildLayout(records, viewport.width, gapSize, minColumnWidth),
    [records, viewport.width, gapSize, minColumnWidth],
  );
  const masonryHeight = layoutItems.reduce((height, item) => Math.max(height, item.bottom + gapSize), gapSize);
  const visibleIndexes = useMemo(() => {
    const minVisible = Math.max(0, viewport.scrollY - OVERSCAN);
    const maxVisible = viewport.scrollY + viewport.height + OVERSCAN;
    const indexes: number[] = [];
    for (let index = 0; index < layoutItems.length; index += 1) {
      const item = layoutItems[index];
      if (!item) continue;
      if (item.bottom < minVisible) continue;
      if (item.top > maxVisible) break;
      indexes.push(index);
    }
    return indexes;
  }, [layoutItems, viewport.scrollY, viewport.height]);

  async function loadMore() {
    if (loadingRef.current || doneRef.current) return;
    loadingRef.current = true;
    try {
      const page = await invoke<ImagePage>("list_images", {
        cursor: cursorRef.current,
        limit: PAGE_SIZE,
      });
      const items = page.items;
      setRecords((current) => {
        const next = [...current, ...items];
        recordsRef.current = next;
        return next;
      });
      cursorRef.current = page.nextCursor;
      const nextDone = !page.nextCursor;
      doneRef.current = nextDone;
      setDone(nextDone);
    } finally {
      loadingRef.current = false;
    }
  }

  async function reloadFirstPage() {
    if (loadingRef.current) return;
    cursorRef.current = null;
    doneRef.current = false;
    setDone(false);
    if (recordsRef.current.length > 0) {
      recordsRef.current = [];
      setRecords([]);
    }
    await loadMore();
  }

  function showPreview(record: ImageRecord) {
    setContextMenu(null);
    if (record.mediaType === "video") {
      pauseTileVideo(record.path);
    }
    const src = convertFileSrc(record.path);
    if (record.mediaType === "video") {
      setPreview({
        type: "video",
        src,
        width: Math.max(1, record.width || 16),
        height: Math.max(1, record.height || 9),
      });
      return;
    }
    setPreview({ type: "image", src, path: record.path });
  }

  const playTileVideo = useCallback((path: string, video: HTMLVideoElement | null) => {
    if (!video) return;
    const playingVideos = playingVideosRef.current;
    if (playingVideos.has(path)) {
      playingVideos.delete(path);
    }
    playingVideos.set(path, video);

    while (playingVideos.size > MAX_PLAYING_TILE_VIDEOS) {
      const oldest = playingVideos.entries().next().value;
      if (!oldest) break;
      const [oldestPath, oldestVideo] = oldest;
      playingVideos.delete(oldestPath);
      pauseVideoElement(oldestVideo);
    }

    video.play().catch((error) => logError(error, "Failed to play gallery video"));
  }, []);

  const pauseTileVideo = useCallback((path: string) => {
    const video = playingVideosRef.current.get(path);
    if (!video) return;
    playingVideosRef.current.delete(path);
    pauseVideoElement(video);
  }, []);

  function handleTileClick(event: React.MouseEvent, record: ImageRecord) {
    if ((event.ctrlKey || event.metaKey) && record.mediaType === "image") {
      event.preventDefault();
      setPreview(null);
      setContextMenu(null);
      toggleReferenceSelection(record);
      return;
    }
    showPreview(record);
  }

  function toggleReferenceSelection(record: ImageRecord) {
    const current = selectedReferenceRecordsRef.current;
    const exists = current.some((item) => item.path === record.path);
    const next = exists
      ? current.filter((item) => item.path !== record.path)
      : [...current, record].slice(0, MAX_REFERENCE_SELECTION);

    selectedReferenceRecordsRef.current = next;
    setSelectedReferenceRecords(next);

    if (!exists && next.length === MAX_REFERENCE_SELECTION) {
      window.requestAnimationFrame(() => {
        editorRef.current?.open(next).catch((error) => logError(error, "Failed to open editor"));
        selectedReferenceRecordsRef.current = [];
        setSelectedReferenceRecords([]);
      });
    }
  }

  function showContextMenu(event: React.MouseEvent, record: ImageRecord) {
    event.preventDefault();
    if ((event.ctrlKey || event.metaKey) && record.mediaType === "image") {
      setPreview(null);
      setContextMenu(null);
      toggleReferenceSelection(record);
      return;
    }
    setPreview(null);
    const menuWidth = 118;
    const menuHeight = 40;
    const left = Math.min(event.clientX, window.innerWidth - menuWidth - 8);
    const top = Math.min(event.clientY, window.innerHeight - menuHeight - 8);
    setContextMenu({
      left: Math.max(8, left),
      top: Math.max(8, top),
      record,
    });
  }

  if (!preferences) {
    return <main className={`gallery-shell theme-${initialTheme}`} />;
  }

  const themeClass = `theme-${preferences.theme === "black" ? "black" : "white"}`;

  return (
    <main
      className={classNames(
        "gallery-shell",
        preferences.hasGap ? "gallery-gap" : "gallery-flush",
        themeClass,
      )}
      onDragStart={(event) => {
        if (!(event.target as Element).closest("#editor-drawer")) event.preventDefault();
      }}
    >
      <section className="masonry" style={{ height: `${masonryHeight}px` }}>
        {visibleIndexes.map((index) => {
          const record = records[index];
          const layout = layoutItems[index];
          if (!record || !layout) return null;
          const selected = selectedReferenceRecords.some((item) => item.path === record.path);
          return (
            <button
              className={classNames("image-tile", selected && "is-selected")}
              key={record.path}
              type="button"
              style={{
                width: `${layout.width}px`,
                height: `${layout.height}px`,
                transform: `translate3d(${layout.left}px, ${layout.top}px, 0)`,
              }}
              onClick={(event) => handleTileClick(event, record)}
              onContextMenu={(event) => showContextMenu(event, record)}
            >
              {record.mediaType === "video" ? (
                <VideoTile
                  record={record}
                  onPlay={playTileVideo}
                  onRemove={pauseTileVideo}
                />
              ) : (
                <TileImage record={record} />
              )}
              {selected ? (
                <span className="image-tile-badge image-tile-selection-mark">
                  <Icons.PuzzlePiece />
                </span>
              ) : null}
            </button>
          );
        })}
      </section>
      <div className="gallery-spacer" hidden={records.length > 0} />
      <div
        ref={sentinelRef}
        id="sentinel"
        aria-hidden="true"
        hidden={done}
        style={{ height: done ? "1px" : `${Math.max(1, Math.min(240, viewport.height * 0.25))}px` }}
      />

      <PreviewOverlay preview={preview} onClose={() => setPreview(null)} />

      {contextMenu ? (
        <div className="context-menu" id="context-menu" style={{ left: contextMenu.left, top: contextMenu.top }}>
          <button
            type="button"
            disabled={contextMenu.record.mediaType !== "image"}
            onClick={() => {
              const record = contextMenu.record;
              setContextMenu(null);
              editorRef.current?.open(record).catch((error) => logError(error, "Failed to open editor"));
              selectedReferenceRecordsRef.current = [];
              setSelectedReferenceRecords([]);
            }}
          >
            <Icons.PaintBrush />
            <span>编辑</span>
          </button>
        </div>
      ) : null}

      <EditorDrawer
        ref={editorRef}
        readImageDataUri={(path) => invoke("read_image_data_uri", { path })}
        pickReferenceImages={() => invoke<PickedImage[]>("pick_xai_reference_images")}
        editImage={(payload) => invoke<XaiEditResult>("edit_image_with_xai", payload)}
        onPreviewAttachment={(attachment) => setPreview({ type: "image", src: attachment.dataUrl || convertFileSrc(attachment.path), path: attachment.path })}
        onToggle={(nextOpen) => {
          document.body.classList.toggle("editing", nextOpen);
          if (nextOpen) {
            selectedReferenceRecordsRef.current = [];
            setSelectedReferenceRecords([]);
          }
        }}
        onError={(error, label) => logError(error, label)}
      />
    </main>
  );
}

function VideoTile({
  record,
  onPlay,
  onRemove,
}: {
  record: ImageRecord;
  onPlay: (path: string, video: HTMLVideoElement | null) => void;
  onRemove: (path: string) => void;
}) {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    return () => onRemove(record.path);
  }, [record.path, onRemove]);

  useEffect(() => {
    setFailed(false);
  }, [record.path]);

  return (
    <>
      <video
        ref={videoRef}
        src={convertFileSrc(record.path)}
        muted
        loop
        playsInline
        controls={false}
        preload="metadata"
        draggable={false}
        className={classNames("tile-media", failed && "is-error")}
        onLoadedMetadata={(event) => primeTileVideoFrame(event.currentTarget)}
        onLoadedData={() => setFailed(false)}
        onSeeked={() => setFailed(false)}
        onError={() => setFailed(true)}
      />
      {failed ? <MediaPlaceholder path={record.path} /> : null}
      <span
        className="video-tile-hover-target"
        onMouseEnter={() => onPlay(record.path, videoRef.current)}
      >
        <span className="image-tile-badge video-tile-kind-mark">
          <Icons.VideoCamera />
        </span>
      </span>
    </>
  );
}

function PreviewOverlay({ preview, onClose }: { preview: PreviewState; onClose: () => void }) {
  if (!preview) return null;
  if (preview.type === "video") {
    return (
      <div className="preview video-preview" onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}>
        <video
          id="preview-video"
          controls
          autoPlay
          loop
          draggable={false}
          src={preview.src}
          style={{
            ["--video-ratio" as string]: String(preview.width / preview.height),
            ["--video-aspect" as string]: `${preview.width} / ${preview.height}`,
          }}
        />
      </div>
    );
  }

  return (
    <div className="preview image-preview" onClick={onClose}>
      <figure>
        <PreviewImage src={preview.src} path={preview.path} />
      </figure>
    </div>
  );
}

function pauseVideoElement(video: HTMLVideoElement) {
  video.pause();
}

function primeTileVideoFrame(video: HTMLVideoElement) {
  if (!Number.isFinite(video.duration) || video.duration <= 0) return;
  if (video.currentTime > 0) return;
  const targetTime = Math.min(TILE_VIDEO_PREVIEW_TIME, Math.max(0.001, video.duration * 0.02));
  try {
    video.currentTime = targetTime;
  } catch (error) {
    logError(error, "Failed to seek gallery video preview frame");
  }
}
