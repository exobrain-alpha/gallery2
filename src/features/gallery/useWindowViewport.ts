import { useEffect, useState } from "react";

const RESIZE_DEBOUNCE_MS = 180;

export interface GalleryViewportState {
  scrollY: number;
  width: number;
  height: number;
}

export interface WindowSizeState {
  width: number;
  height: number;
}

function readGalleryViewport(): GalleryViewportState {
  return {
    scrollY: window.scrollY,
    width: window.innerWidth,
    height: window.innerHeight,
  };
}

function readWindowSize(): WindowSizeState {
  return {
    width: window.innerWidth,
    height: window.innerHeight,
  };
}

function sameGalleryViewport(current: GalleryViewportState, next: GalleryViewportState) {
  return current.scrollY === next.scrollY && current.width === next.width && current.height === next.height;
}

function sameWindowSize(current: WindowSizeState, next: WindowSizeState) {
  return current.width === next.width && current.height === next.height;
}

export function useGalleryViewport() {
  const [viewport, setViewport] = useState<GalleryViewportState>(() => readGalleryViewport());
  const [isResizing, setIsResizing] = useState(false);

  useEffect(() => {
    let scrollFrame = 0;
    let resizeTimer = 0;
    let resizeActive = false;

    const setResizeActive = (active: boolean) => {
      if (resizeActive === active) return;
      resizeActive = active;
      setIsResizing(active);
    };

    const updateScroll = () => {
      if (scrollFrame) return;
      scrollFrame = window.requestAnimationFrame(() => {
        scrollFrame = 0;
        const scrollY = window.scrollY;
        setViewport((current) => current.scrollY === scrollY ? current : { ...current, scrollY });
      });
    };

    const commitResize = () => {
      resizeTimer = 0;
      setViewport((current) => {
        const next = readGalleryViewport();
        return sameGalleryViewport(current, next) ? current : next;
      });
      setResizeActive(false);
    };

    const updateSize = () => {
      setResizeActive(true);
      if (resizeTimer) window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(commitResize, RESIZE_DEBOUNCE_MS);
    };

    window.addEventListener("scroll", updateScroll, { passive: true });
    window.addEventListener("resize", updateSize, { passive: true });
    return () => {
      if (scrollFrame) window.cancelAnimationFrame(scrollFrame);
      if (resizeTimer) window.clearTimeout(resizeTimer);
      window.removeEventListener("scroll", updateScroll);
      window.removeEventListener("resize", updateSize);
    };
  }, []);

  return { viewport, isResizing };
}

export function useDebouncedWindowSize() {
  const [size, setSize] = useState<WindowSizeState>(() => readWindowSize());
  const [isResizing, setIsResizing] = useState(false);

  useEffect(() => {
    let resizeTimer = 0;
    let resizeActive = false;

    const setResizeActive = (active: boolean) => {
      if (resizeActive === active) return;
      resizeActive = active;
      setIsResizing(active);
    };

    const commitResize = () => {
      resizeTimer = 0;
      setSize((current) => {
        const next = readWindowSize();
        return sameWindowSize(current, next) ? current : next;
      });
      setResizeActive(false);
    };

    const updateSize = () => {
      setResizeActive(true);
      if (resizeTimer) window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(commitResize, RESIZE_DEBOUNCE_MS);
    };

    window.addEventListener("resize", updateSize, { passive: true });
    return () => {
      if (resizeTimer) window.clearTimeout(resizeTimer);
      window.removeEventListener("resize", updateSize);
    };
  }, []);

  return { size, isResizing };
}
