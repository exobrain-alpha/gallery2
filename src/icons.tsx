import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement>;

function SvgIcon({ children, ...props }: IconProps) {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" {...props}>
      {children}
    </svg>
  );
}

export function FolderIcon(props: IconProps) {
  return (
    <SvgIcon {...props}>
      <path d="M19.5 21a3 3 0 0 0 3-3v-4.5a3 3 0 0 0-3-3h-15a3 3 0 0 0-3 3V18a3 3 0 0 0 3 3h15ZM1.5 10.146V6a3 3 0 0 1 3-3h5.379a2.25 2.25 0 0 1 1.59.659l2.122 2.121c.14.141.331.22.53.22H19.5a3 3 0 0 1 3 3v1.146A4.483 4.483 0 0 0 19.5 9h-15a4.483 4.483 0 0 0-3 1.146Z" />
    </SvgIcon>
  );
}

export function PaintBrushIcon(props: IconProps) {
  return (
    <SvgIcon {...props}>
      <path d="M16.862 4.487a1.875 1.875 0 0 1 2.651 2.651L10.582 16.07a4.5 4.5 0 0 1-1.897 1.13L6 18l.8-2.685a4.5 4.5 0 0 1 1.13-1.897l8.932-8.931Z" />
      <path d="M18 14.25a.75.75 0 0 1 .75.75v2.25A3.75 3.75 0 0 1 15 21H6.75A3.75 3.75 0 0 1 3 17.25V9A3.75 3.75 0 0 1 6.75 5.25H9a.75.75 0 0 1 0 1.5H6.75A2.25 2.25 0 0 0 4.5 9v8.25a2.25 2.25 0 0 0 2.25 2.25H15a2.25 2.25 0 0 0 2.25-2.25V15a.75.75 0 0 1 .75-.75Z" />
    </SvgIcon>
  );
}

export function ArrowUpIcon(props: IconProps) {
  return (
    <SvgIcon {...props}>
      <path fillRule="evenodd" d="M11.47 2.47a.75.75 0 0 1 1.06 0l7.5 7.5a.75.75 0 1 1-1.06 1.06l-6.22-6.22V21a.75.75 0 0 1-1.5 0V4.81l-6.22 6.22a.75.75 0 1 1-1.06-1.06l7.5-7.5Z" clipRule="evenodd" />
    </SvgIcon>
  );
}

export function PlusIcon(props: IconProps) {
  return (
    <SvgIcon {...props}>
      <path fillRule="evenodd" d="M12 3.75a.75.75 0 0 1 .75.75v6.75h6.75a.75.75 0 0 1 0 1.5h-6.75v6.75a.75.75 0 0 1-1.5 0v-6.75H4.5a.75.75 0 0 1 0-1.5h6.75V4.5A.75.75 0 0 1 12 3.75Z" clipRule="evenodd" />
    </SvgIcon>
  );
}

export function XMarkIcon(props: IconProps) {
  return (
    <SvgIcon {...props}>
      <path fillRule="evenodd" d="M5.47 5.47a.75.75 0 0 1 1.06 0L12 10.94l5.47-5.47a.75.75 0 1 1 1.06 1.06L13.06 12l5.47 5.47a.75.75 0 1 1-1.06 1.06L12 13.06l-5.47 5.47a.75.75 0 1 1-1.06-1.06L10.94 12 5.47 6.53a.75.75 0 0 1 0-1.06Z" clipRule="evenodd" />
    </SvgIcon>
  );
}

export function ArrowPathIcon(props: IconProps) {
  return (
    <SvgIcon {...props}>
      <path fillRule="evenodd" d="M4.755 10.059a7.5 7.5 0 0 1 12.548-3.364l1.903 1.903h-3.183a.75.75 0 1 0 0 1.5h4.992a.75.75 0 0 0 .75-.75V4.356a.75.75 0 0 0-1.5 0v3.18l-1.9-1.9A9 9 0 0 0 3.306 9.67a.75.75 0 1 0 1.45.388Zm15.408 3.352a.75.75 0 0 0-.919.53 7.5 7.5 0 0 1-12.548 3.364l-1.902-1.903h3.183a.75.75 0 0 0 0-1.5H2.984a.75.75 0 0 0-.75.75v4.992a.75.75 0 0 0 1.5 0v-3.18l1.9 1.9a9 9 0 0 0 15.059-4.035.75.75 0 0 0-.53-.918Z" clipRule="evenodd" />
    </SvgIcon>
  );
}

export function ClipboardDocumentIcon(props: IconProps) {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" {...props}>
      <path fillRule="evenodd" d="M4 2a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2zm2-1a1 1 0 0 0-1 1v8a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V2a1 1 0 0 0-1-1zM2 5a1 1 0 0 0-1 1v8a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1v-1h1v1a2 2 0 0 1-2 2H2a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h1v1z" />
    </svg>
  );
}

export function ArrowTopRightIcon(props: IconProps) {
  return (
    <SvgIcon {...props}>
      <path fillRule="evenodd" d="M15.75 2.25H21A.75.75 0 0 1 21.75 3v5.25a.75.75 0 0 1-1.5 0V4.81L9.53 15.53a.75.75 0 0 1-1.06-1.06L19.19 3.75h-3.44a.75.75 0 0 1 0-1.5Zm-10.5 4.5a1.5 1.5 0 0 0-1.5 1.5v10.5a1.5 1.5 0 0 0 1.5 1.5h10.5a1.5 1.5 0 0 0 1.5-1.5V12a.75.75 0 0 1 1.5 0v6.75a3 3 0 0 1-3 3H5.25a3 3 0 0 1-3-3V8.25a3 3 0 0 1 3-3H12a.75.75 0 0 1 0 1.5H5.25Z" clipRule="evenodd" />
    </SvgIcon>
  );
}

export function CheckBadgeIcon(props: IconProps) {
  return (
    <SvgIcon {...props}>
      <path fillRule="evenodd" d="M8.603 3.799A4.49 4.49 0 0 1 12 2.25c1.357 0 2.573.6 3.397 1.549a4.49 4.49 0 0 1 3.498 1.307 4.491 4.491 0 0 1 1.307 3.497A4.49 4.49 0 0 1 21.75 12a4.49 4.49 0 0 1-1.548 3.397 4.491 4.491 0 0 1-1.307 3.497 4.491 4.491 0 0 1-3.498 1.307A4.49 4.49 0 0 1 12 21.75a4.49 4.49 0 0 1-3.397-1.549 4.49 4.49 0 0 1-3.498-1.307 4.491 4.491 0 0 1-1.307-3.497A4.49 4.49 0 0 1 2.25 12c0-1.357.6-2.573 1.548-3.397a4.49 4.49 0 0 1 1.307-3.497 4.49 4.49 0 0 1 3.498-1.307Zm7.007 6.387a.75.75 0 1 0-1.22-.872l-3.236 4.53L9.53 12.22a.75.75 0 0 0-1.06 1.06l2.25 2.25a.75.75 0 0 0 1.14-.094l3.75-5.25Z" clipRule="evenodd" />
    </SvgIcon>
  );
}

export const Icons = {
  Folder: FolderIcon,
  PaintBrush: PaintBrushIcon,
  ArrowUp: ArrowUpIcon,
  Plus: PlusIcon,
  XMark: XMarkIcon,
  ArrowPath: ArrowPathIcon,
  ClipboardDocument: ClipboardDocumentIcon,
  ArrowTopRight: ArrowTopRightIcon,
  CheckBadge: CheckBadgeIcon,
};
