import type { MutableRefObject } from "react";
import type { ResizablePanelHandle } from "@ora/ui";

interface PanelWidthAnimationOptions {
  animationRef: MutableRefObject<number | null>;
  duration: number;
  panel: ResizablePanelHandle | null;
  targetWidth: number;
  onCollapsed?: () => void;
  onComplete?: () => void;
}

/** Stops a scripted panel settle so direct pointer input always takes priority. */
export function cancelPanelWidthAnimation(
  animationRef: MutableRefObject<number | null>,
): void {
  if (animationRef.current === null) {
    return;
  }
  window.cancelAnimationFrame(animationRef.current);
  animationRef.current = null;
}

/** Settles a panel width with an interruptible ease-out and accessible motion fallback. */
export function animatePanelWidth({
  animationRef,
  duration,
  onCollapsed,
  onComplete,
  panel,
  targetWidth,
}: PanelWidthAnimationOptions): void {
  if (panel === null) {
    return;
  }
  cancelPanelWidthAnimation(animationRef);
  let startWidth = 0;
  try {
    startWidth = panel.getSize().inPixels;
  } catch {
    // The panel can detach from its group while a settle is being queued
    // (e.g. the host unmounted mid-animation); jumping to the target then is
    // still the closest to the requested end state.
    panel.resize(targetWidth);
    onComplete?.();
    return;
  }
  const reducedMotion =
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

  const finish = (): void => {
    if (targetWidth === 0) {
      // The primitive tracks collapsed state separately from a zero resize.
      panel.collapse();
      onCollapsed?.();
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
    try {
      panel.resize(startWidth + (targetWidth - startWidth) * easedProgress);
    } catch {
      // The group registry can drop the panel before the next frame (unmount or
      // jsdom teardown); a detached panel has no width left to animate.
      animationRef.current = null;
      return;
    }
    if (progress < 1) {
      animationRef.current = window.requestAnimationFrame(animate);
      return;
    }
    animationRef.current = null;
    finish();
  };
  animationRef.current = window.requestAnimationFrame(animate);
}
