import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { ImageRecord } from "../../types";
import { classNames } from "../../utils";

export function TileImage({ record }: { record: ImageRecord }) {
  const [sourceKind, setSourceKind] = useState<"display" | "original">("display");
  const [failed, setFailed] = useState(false);
  const displayPath = record.displayPath || record.path;
  const src = convertFileSrc(sourceKind === "display" ? displayPath : record.path);

  useEffect(() => {
    setSourceKind("display");
    setFailed(false);
  }, [displayPath, record.path]);

  function handleError() {
    if (sourceKind === "display" && displayPath !== record.path) {
      setSourceKind("original");
      setFailed(false);
      return;
    }
    setFailed(true);
  }

  return (
    <>
      <img
        className={classNames("tile-media", failed && "is-error")}
        loading="lazy"
        decoding="async"
        draggable={false}
        src={src}
        alt=""
        onLoad={() => setFailed(false)}
        onError={handleError}
      />
      {failed ? <MediaPlaceholder path={record.path} /> : null}
    </>
  );
}

export function MediaPlaceholder({ path }: { path: string }) {
  return (
    <span className="media-placeholder" aria-hidden="true" title={path}>
      <span className="media-placeholder-title">加载失败</span>
    </span>
  );
}

export function PreviewImage({ src, path }: { src: string; path?: string }) {
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [src]);

  return (
    <>
      <img
        id="preview-image"
        className={classNames("preview-media", failed && "is-error")}
        src={src}
        alt=""
        draggable={false}
        onLoad={() => setFailed(false)}
        onError={() => setFailed(true)}
      />
      {failed ? <MediaPlaceholder path={path || src} /> : null}
    </>
  );
}
