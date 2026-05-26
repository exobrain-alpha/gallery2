import type { ImageRecord } from "../../types";

export interface LayoutItem {
  column: number;
  left: number;
  top: number;
  width: number;
  height: number;
  bottom: number;
}

export function buildLayout(records: ImageRecord[], viewportWidth: number, gapSize: number, minColumnWidth: number): LayoutItem[] {
  const columnCount = getColumnCount(viewportWidth, gapSize, minColumnWidth);
  const columnWidth = getColumnWidth(viewportWidth, gapSize, minColumnWidth);
  const sideOffset = gapSize;
  const columnHeights = new Array(columnCount).fill(gapSize);
  const layoutItems: LayoutItem[] = [];

  for (let index = 0; index < records.length; index += 1) {
    const record = records[index];
    let shortestColumn = 0;
    for (let column = 1; column < columnCount; column += 1) {
      if (columnHeights[column] < columnHeights[shortestColumn]) shortestColumn = column;
    }

    const rawLeft = sideOffset + shortestColumn * (columnWidth + gapSize);
    const rawRight = rawLeft + columnWidth;
    const left = gapSize === 0 ? Math.round(rawLeft) : rawLeft;
    const itemWidth = gapSize === 0 ? Math.max(1, Math.round(rawRight) - left) : columnWidth;
    const height = getItemHeight(record, itemWidth);
    const top = columnHeights[shortestColumn];
    const bottom = top + height;

    layoutItems[index] = {
      column: shortestColumn,
      left,
      top,
      width: itemWidth,
      height,
      bottom,
    };
    columnHeights[shortestColumn] = bottom + gapSize;
  }

  return layoutItems;
}

export function getColumnCount(viewportWidth: number, gapSize: number, minColumnWidth: number) {
  const width = Math.max(1, viewportWidth - gapSize * 2);
  return Math.max(1, Math.floor((width + gapSize) / (minColumnWidth + gapSize)));
}

export function getColumnWidth(viewportWidth: number, gapSize: number, minColumnWidth: number) {
  const columnCount = getColumnCount(viewportWidth, gapSize, minColumnWidth);
  const containerWidth = Math.max(1, viewportWidth - gapSize * 2);
  return Math.max(1, (containerWidth - gapSize * (columnCount - 1)) / columnCount);
}

export function getItemHeight(record: ImageRecord, width: number) {
  const safeHeight = Math.max(1, record.height || 1);
  const safeWidth = Math.max(1, record.width || 1);
  return Math.max(1, Math.round((width * safeHeight) / safeWidth));
}
