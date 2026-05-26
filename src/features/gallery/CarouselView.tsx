import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, WheelEvent } from "react";
import type { GalleryPreferences, ImageRecord } from "../../types";
import { classNames, logError, mediaName, setPageBackground, storeGalleryTheme, storedGalleryTheme } from "../../utils";
import { getColumnCount, getColumnWidth, getItemHeight } from "./layout";

const RANDOM_IMAGE_LIMIT = 360;
const AUTO_SCROLL_SPEED = 22;
const DOWNWARD_SCROLL_FACTOR = 0.88;
const UPWARD_SCROLL_FACTOR = 1.12;
const WHEEL_SCROLL_FACTOR = 0.65;
const MAX_WHEEL_DELTA = 120;
const RESUME_DELAY_MS = 1400;
const WHEEL_LINE_HEIGHT = 16;

interface ViewportState {
  width: number;
  height: number;
}

interface CarouselColumn {
  records: ImageRecord[];
  heights: number[];
  cycleHeight: number;
  direction: 1 | -1;
}

type PreviewState = { type: "image"; src: string } | null;

export function CarouselView() {
  const [preferences, setPreferences] = useState<GalleryPreferences | null>(null);
  const initialTheme = useMemo(() => storedGalleryTheme(), []);
  const [records, setRecords] = useState<ImageRecord[]>([]);
  const [preview, setPreview] = useState<PreviewState>(null);
  const [viewport, setViewport] = useState<ViewportState>(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
  }));
  const trackRefs = useRef<Array<HTMLDivElement | null>>([]);
  const offsetsRef = useRef<number[]>([]);
  const pauseUntilRefs = useRef<number[]>([]);

  useEffect(() => {
    document.body.classList.add("carouseling");
    setPageBackground(initialTheme === "black" ? "#1a1b1e" : "#ffffff");
    invoke<GalleryPreferences>("get_gallery_preferences")
      .then((loadedPreferences) => {
        setPreferences(loadedPreferences);
        storeGalleryTheme(loadedPreferences.theme);
        setPageBackground(loadedPreferences.theme === "black" ? "#1a1b1e" : "#ffffff");
      })
      .catch((error) => logError(error, "Failed to load gallery preferences"));
    return () => document.body.classList.remove("carouseling");
  }, [initialTheme]);

  useEffect(() => {
    let frame = 0;
    const updateViewport = () => {
      if (frame) return;
      frame = window.requestAnimationFrame(() => {
        frame = 0;
        setViewport({
          width: window.innerWidth,
          height: window.innerHeight,
        });
      });
    };

    window.addEventListener("resize", updateViewport, { passive: true });
    return () => {
      if (frame) window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", updateViewport);
    };
  }, []);

  useEffect(() => {
    loadRandomImages().catch((error) => logError(error, "Failed to load carousel images"));
  }, []);

  useEffect(() => {
    const reloadCarousel = () => {
      loadRandomImages().catch((error) => logError(error, "Failed to reload carousel images"));
    };
    window.addEventListener("gallery:reload", reloadCarousel);
    return () => window.removeEventListener("gallery:reload", reloadCarousel);
  }, []);

  useEffect(() => {
    document.body.classList.toggle("previewing", preview !== null);
    return () => document.body.classList.remove("previewing");
  }, [preview]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setPreview(null);
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const gapSize = preferences?.hasGap ? 12 : 0;
  const minColumnWidth = Math.min(600, Math.max(100, preferences?.minColumnWidth || 280));
  const columnCount = getColumnCount(viewport.width, gapSize, minColumnWidth);
  const columnWidth = getColumnWidth(viewport.width, gapSize, minColumnWidth);
  const columns = useMemo(
    () => buildCarouselColumns(records, columnCount, columnWidth, gapSize, viewport.height),
    [records, columnCount, columnWidth, gapSize, viewport.height],
  );

  useEffect(() => {
    for (let index = 0; index < columns.length; index += 1) {
      const column = columns[index];
      if (!column || column.cycleHeight <= 0) continue;
      offsetsRef.current[index] = column.cycleHeight;
      applyColumnOffset(index, column.cycleHeight);
    }
  }, [columns]);

  useEffect(() => {
    let animationFrame = 0;
    let previousTime = performance.now();

    const animate = (time: number) => {
      const deltaSeconds = Math.min(0.05, Math.max(0, (time - previousTime) / 1000));
      previousTime = time;

      for (let index = 0; index < columns.length; index += 1) {
        const column = columns[index];
        if (!column || column.cycleHeight <= 0 || time < (pauseUntilRefs.current[index] || 0)) {
          continue;
        }

        advanceColumnOffset(index, column, autoScrollDelta(column, deltaSeconds));
      }

      animationFrame = window.requestAnimationFrame(animate);
    };

    animationFrame = window.requestAnimationFrame(animate);
    return () => window.cancelAnimationFrame(animationFrame);
  }, [columns]);

  async function loadRandomImages() {
    const items = await invoke<ImageRecord[]>("list_random_images", { limit: RANDOM_IMAGE_LIMIT });
    setRecords(items);
  }

  function pauseColumnAutoScroll(columnIndex: number) {
    pauseUntilRefs.current[columnIndex] = performance.now() + RESUME_DELAY_MS;
  }

  function applyColumnOffset(columnIndex: number, offset: number) {
    const track = trackRefs.current[columnIndex];
    if (!track) return;
    track.style.transform = `translate3d(0, ${-offset}px, 0)`;
  }

  function normalizeColumnOffset(offset: number, cycleHeight: number) {
    if (cycleHeight <= 0) return 0;
    const minOffset = 0;
    const maxOffset = cycleHeight * 2;
    let nextOffset = offset;
    while (nextOffset >= maxOffset) nextOffset -= cycleHeight;
    while (nextOffset <= minOffset) nextOffset += cycleHeight;
    return nextOffset;
  }

  function advanceColumnOffset(columnIndex: number, column: CarouselColumn, delta: number) {
    const nextOffset = normalizeColumnOffset(
      (offsetsRef.current[columnIndex] ?? column.cycleHeight) + delta,
      column.cycleHeight,
    );
    offsetsRef.current[columnIndex] = nextOffset;
    applyColumnOffset(columnIndex, nextOffset);
  }

  function autoScrollDelta(column: CarouselColumn, deltaSeconds: number) {
    const directionFactor = column.direction > 0 ? DOWNWARD_SCROLL_FACTOR : UPWARD_SCROLL_FACTOR;
    return column.direction * AUTO_SCROLL_SPEED * directionFactor * deltaSeconds;
  }

  function handleColumnWheel(event: WheelEvent, columnIndex: number) {
    const column = columns[columnIndex];
    if (!column || column.cycleHeight <= 0) return;
    event.preventDefault();
    advanceColumnOffset(columnIndex, column, normalizedWheelDelta(event) * WHEEL_SCROLL_FACTOR);
    pauseColumnAutoScroll(columnIndex);
  }

  function showPreview(record: ImageRecord) {
    setPreview({ type: "image", src: convertFileSrc(record.path) });
  }

  if (!preferences) {
    return <main className={`gallery-shell carousel-shell theme-${initialTheme}`} />;
  }

  const themeClass = `theme-${preferences.theme === "black" ? "black" : "white"}`;

  return (
    <main
      className={classNames(
        "gallery-shell",
        "carousel-shell",
        preferences.hasGap ? "gallery-gap" : "gallery-flush",
        themeClass,
      )}
      style={{ "--carousel-gap": `${gapSize}px` } as CSSProperties}
      onDragStart={(event) => event.preventDefault()}
      onContextMenu={(event) => event.preventDefault()}
    >
      <section className="carousel-columns">
        {columns.map((column, columnIndex) => (
          <div
            className="carousel-column"
            key={columnIndex}
            style={{ width: `${columnWidth}px` }}
            onWheel={(event) => handleColumnWheel(event, columnIndex)}
          >
            <div
              className="carousel-track"
              ref={(element) => {
                trackRefs.current[columnIndex] = element;
              }}
            >
              {[0, 1, 2].map((copyIndex) => (
                <div className="carousel-cycle" key={copyIndex}>
                  {column.records.map((record, recordIndex) => (
                    <div
                      className="image-tile carousel-tile"
                      key={`${copyIndex}-${record.path}-${recordIndex}`}
                      style={{
                        width: `${columnWidth}px`,
                        height: `${column.heights[recordIndex] || 1}px`,
                      }}
                      onClick={() => showPreview(record)}
                    >
                      <img loading="lazy" decoding="async" draggable={false} src={convertFileSrc(record.displayPath || record.path)} alt={mediaName(record.path)} />
                    </div>
                  ))}
                </div>
              ))}
            </div>
          </div>
        ))}
      </section>
      <PreviewOverlay preview={preview} onClose={() => setPreview(null)} />
    </main>
  );
}

function PreviewOverlay({ preview, onClose }: { preview: PreviewState; onClose: () => void }) {
  if (!preview) return null;
  return (
    <div className="preview image-preview" onClick={onClose}>
      <figure>
        <img id="preview-image" src={preview.src} alt="" draggable={false} />
      </figure>
    </div>
  );
}

function buildCarouselColumns(
  allRecords: ImageRecord[],
  columnCount: number,
  columnWidth: number,
  gapSize: number,
  viewportHeight: number,
): CarouselColumn[] {
  const buckets = Array.from({ length: columnCount }, () => [] as ImageRecord[]);
  const bucketHeights = new Array(columnCount).fill(0);
  for (const record of allRecords) {
    let shortestColumn = 0;
    for (let columnIndex = 1; columnIndex < columnCount; columnIndex += 1) {
      if (bucketHeights[columnIndex] < bucketHeights[shortestColumn]) {
        shortestColumn = columnIndex;
      }
    }
    buckets[shortestColumn]?.push(record);
    bucketHeights[shortestColumn] += getItemHeight(record, columnWidth) + gapSize;
  }

  return buckets.map((bucket, columnIndex) => {
    const records = buildLoopRecords(bucket, allRecords, columnIndex, columnCount, columnWidth, gapSize, viewportHeight);
    const heights = records.map((record) => getItemHeight(record, columnWidth));
    const cycleHeight = heights.reduce((total, height) => total + height + gapSize, 0);

    return {
      records,
      heights,
      cycleHeight,
      direction: columnIndex % 2 === 0 ? 1 : -1,
    };
  });
}

function buildLoopRecords(
  bucket: ImageRecord[],
  fallbackRecords: ImageRecord[],
  columnIndex: number,
  columnCount: number,
  columnWidth: number,
  gapSize: number,
  viewportHeight: number,
) {
  if (fallbackRecords.length === 0) return [];

  const loopRecords = bucket.length > 0 ? [...bucket] : [fallbackRecords[columnIndex % fallbackRecords.length]];
  const targetHeight = Math.max(viewportHeight * 1.35, viewportHeight + columnWidth);
  let height = loopRecords.reduce((total, record) => total + getItemHeight(record, columnWidth) + gapSize, 0);
  let cursor = columnIndex;
  const maxLoopRecords = Math.max(fallbackRecords.length * 6, columnCount * 6, 12);

  while (height < targetHeight && loopRecords.length < maxLoopRecords) {
    const record = fallbackRecords[cursor % fallbackRecords.length];
    loopRecords.push(record);
    height += getItemHeight(record, columnWidth) + gapSize;
    cursor += Math.max(1, columnCount);
  }

  return loopRecords;
}

function normalizedWheelDelta(event: WheelEvent) {
  const unitDelta =
    event.deltaMode === 1
      ? event.deltaY * WHEEL_LINE_HEIGHT
      : event.deltaMode === 2
        ? event.deltaY * window.innerHeight
        : event.deltaY;
  const magnitude = Math.min(Math.abs(unitDelta), MAX_WHEEL_DELTA);
  return Math.sign(unitDelta) * magnitude;
}
