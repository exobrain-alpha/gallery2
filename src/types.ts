export type MediaType = "image" | "video";

export interface ImageRecord {
  path: string;
  mediaType: MediaType;
  width: number;
  height: number;
  modified: number;
  size: number;
}

export interface SettingsState {
  paths: string[];
  imageCount: number;
  dbPath: string;
  generatedContentDir: string;
  xaiKey: string;
  galleryMode: string;
  galleryHasGap: boolean;
  galleryTheme: "black" | "white";
  minColumnWidth: number;
}

export interface GalleryPreferences {
  hasGap: boolean;
  theme: "black" | "white";
  minColumnWidth: number;
}

export interface ScanSummary {
  indexed: number;
  skipped: number;
  removed: number;
  total: number;
}

export interface DedupeSummary {
  checked: number;
  duplicates: number;
  moved: number;
  skipped: number;
  total: number;
  maxFileSize: number;
}

export interface ExtensionRepairSummary {
  repaired: number;
  skipped: number;
  total: number;
}

export interface PickedImage {
  path: string;
  dataUri: string;
}

export interface XaiEditResult {
  path: string;
  paths: string[];
  response: unknown;
}

export interface Attachment {
  path: string;
  label: string;
  dataUrl?: string;
  dataUri?: string;
}
