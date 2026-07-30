import type { MutableRefObject } from "react";
import type { ResizablePanelHandle } from "@ora/ui";

interface AnimateWorkflowPanelOptions {
  animationRef: MutableRefObject<number | null>;
  duration: number;
  onCollapsed: () => void;
  onComplete?: () => void;
  panel: ResizablePanelHandle | null;
  targetWidth: number;
}

/** Stops a scripted panel settle so direct pointer input always takes priority. */
export function cancelWorkflowPanelAnimation(
  animationRef: MutableRefObject<number | null>,
): void {
  if (animationRef.current === null) {
    return;
  }
  window.cancelAnimationFrame(animationRef.current);
  animationRef.current = null;
}

/** Settles a panel width with an interruptible ease-out and accessible motion fallback. */
export function animateWorkflowPanel({
  animationRef,
  duration,
  onCollapsed,
  onComplete,
  panel,
  targetWidth,
}: AnimateWorkflowPanelOptions): void {
  if (panel === null) {
    return;
  }
  cancelWorkflowPanelAnimation(animationRef);
  const startWidth = panel.getSize().inPixels;
  const reducedMotion =
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

  const finish = (): void => {
    if (targetWidth === 0) {
      // The primitive tracks collapsed state separately from a zero resize.
      panel.collapse();
      onCollapsed();
    }
    onComplete?.();
  };

  if (reducedMotion || Math.abs(startWidth - targetWidth) < 1) {
    if (targetWidth !== 0) {
      panel.resize(targetWidth);
    }
    finish();
    return;
  }

  const startedAt = window.performance.now();
  const animate = (now: number): void => {
    const progress = Math.min(1, (now - startedAt) / duration);
    const easedProgress = 1 - (1 - progress) ** 3;
    panel.resize(startWidth + (targetWidth - startWidth) * easedProgress);
    if (progress < 1) {
      animationRef.current = window.requestAnimationFrame(animate);
      return;
    }
    animationRef.current = null;
    finish();
  };
  animationRef.current = window.requestAnimationFrame(animate);
}
