import type { DedupeSummary, ExtensionRepairSummary, ScanSummary } from "./types";

const GALLERY_THEME_STORAGE_KEY = "gallery.theme";

export function storedGalleryTheme() {
  const params = new URLSearchParams(window.location.search);
  const queryTheme = params.get("theme");
  if (queryTheme === "black" || queryTheme === "white") {
    return queryTheme;
  }

  try {
    return window.localStorage.getItem(GALLERY_THEME_STORAGE_KEY) === "black" ? "black" : "white";
  } catch {
    return "white";
  }
}

export function storeGalleryTheme(theme: "black" | "white") {
  try {
    window.localStorage.setItem(GALLERY_THEME_STORAGE_KEY, theme);
  } catch {
    // Ignore storage failures; the backend preference remains authoritative.
  }
}

export function setPageBackground(color: string) {
  document.documentElement.style.background = color;
  document.body.style.background = color;
}

export function mediaName(path: string) {
  return path.split(/[\\/]/).pop() || "image";
}

export function formatCount(value: number) {
  return Number(value || 0).toLocaleString("zh-CN");
}

export function formatScanSummary(summary: ScanSummary) {
  const parts = [`完成`, `资源 ${formatCount(summary.indexed)}`];
  if (summary.skipped > 0) parts.push(`异常 ${formatCount(summary.skipped)}`);
  if (summary.removed > 0) parts.push(`清理 ${formatCount(summary.removed)}`);
  return parts.join(" · ");
}

export function formatDedupeSummary(summary: DedupeSummary) {
  const parts = [
    `完成`,
    `范围 ${formatCount(summary.checked)}`,
    `重复 ${formatCount(summary.duplicates)}`,
    `移动 ${formatCount(summary.moved)}`,
  ];
  if (summary.skipped > 0) parts.push(`跳过 ${formatCount(summary.skipped)}`);
  return parts.join(" · ");
}

export function formatRepairSummary(summary: ExtensionRepairSummary) {
  const parts = [`完成`, `修复 ${formatCount(summary.repaired)}`];
  if (summary.skipped > 0) parts.push(`跳过 ${formatCount(summary.skipped)}`);
  return parts.join(" · ");
}

export function rawErrorMessage(error: unknown) {
  if (error instanceof Error) return error.message.trim();
  return String(error ?? "").trim();
}

export function isIgnorableError(error: unknown) {
  const message = rawErrorMessage(error).toLowerCase();
  return (
    !message ||
    message === "null" ||
    message === "undefined" ||
    message.includes("abort") ||
    message.includes("canceled") ||
    message.includes("cancelled")
  );
}

export function formatErrorMessage(error: unknown, fallback = "操作失败") {
  let message = rawErrorMessage(error);
  if (!message) return fallback;
  message = message.replace(/^Error invoking remote method '[^']+':\s*/u, "").trim();
  if (!message) return fallback;
  if (/content moderation|内容审核/i.test(message)) {
    return "内容审核未通过，请调整提示词或参考图后重试";
  }
  if (/billing|balance|credits?|quota|insufficient|payment|funds|spend|usage limit|额度|余额|费用|充值|账单/i.test(message)) {
    return "xAI 账户额度不足，请充值或调整账单后重试";
  }
  if (/xai key/i.test(message) && /未设置|missing|empty/i.test(message)) {
    return "请先设置 xAI Key";
  }
  if (/image_url must either be a base64-encoded image or a URL/i.test(message)) {
    return "参考图读取失败，请重新选择图片后重试";
  }
  return message;
}

export function logError(error: unknown, label = "") {
  if (isIgnorableError(error)) return;
  if (label) {
    console.error(label, error);
    return;
  }
  console.error(error);
}

export function classNames(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(" ");
}

export function uniquePaths(paths: string[]) {
  return Array.from(new Set(paths.map((path) => path.trim()).filter(Boolean)))
    .sort((a, b) => a.localeCompare(b, "zh-CN"))
    .reduce<string[]>((kept, path) => {
      if (!kept.some((parent) => isNestedPath(path, parent))) {
        kept = kept.filter((child) => !isNestedPath(child, path));
        kept.push(path);
      }
      return kept;
    }, []);
}

function isNestedPath(path: string, parent: string) {
  const normalizedPath = path.replace(/[\\/]+$/, "");
  const normalizedParent = parent.replace(/[\\/]+$/, "");
  if (normalizedPath === normalizedParent) return true;
  return (
    normalizedPath.startsWith(`${normalizedParent}/`) ||
    normalizedPath.startsWith(`${normalizedParent}\\`)
  );
}
