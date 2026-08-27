import type { ReactNode, SVGProps } from "react";

interface CanvasToolIconProps extends SVGProps<SVGSVGElement> {
  children: ReactNode;
}

/**
 * Keeps the requested Lucide geometry on an explicit 20px box instead of
 * allowing generic button SVG rules to rescale it. Icon paths are from Lucide
 * v1.34.0 (ISC license).
 */
function CanvasToolIcon({
  children,
  className,
  ...props
}: CanvasToolIconProps) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.3}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`size-5 ${className ?? ""}`}
      aria-hidden="true"
      focusable="false"
      {...props}
    >
      {children}
    </svg>
  );
}

export function StickyNotePlusIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <CanvasToolIcon {...props}>
      <path d="M15 3v5a1 1 0 0 0 1 1h5" />
      <path d="M18 15v6" />
      <path d="M21 12.356V9a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 15 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h7.355" />
      <path d="M21 18h-6" />
    </CanvasToolIcon>
  );
}

export function MousePointer2Icon(props: SVGProps<SVGSVGElement>) {
  return (
    <CanvasToolIcon {...props}>
      <path d="M4.037 4.688a.495.495 0 0 1 .651-.651l16 6.5a.5.5 0 0 1-.063.947l-6.124 1.58a2 2 0 0 0-1.438 1.435l-1.579 6.126a.5.5 0 0 1-.947.063z" />
    </CanvasToolIcon>
  );
}

export function HandIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <CanvasToolIcon {...props}>
      <path d="M18 11V6a2 2 0 0 0-2-2 2 2 0 0 0-2 2" />
      <path d="M14 10V4a2 2 0 0 0-2-2 2 2 0 0 0-2 2v2" />
      <path d="M10 10.5V6a2 2 0 0 0-2-2 2 2 0 0 0-2 2v8" />
      <path d="M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.86-5.99-2.34l-3.6-3.6a2 2 0 0 1 2.83-2.82L7 15" />
    </CanvasToolIcon>
  );
}

export function Grid2x2CheckIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <CanvasToolIcon {...props}>
      <path d="M12 3v17a1 1 0 0 1-1 1H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v6a1 1 0 0 1-1 1H3" />
      <path d="m16 19 2 2 4-4" />
    </CanvasToolIcon>
  );
}
