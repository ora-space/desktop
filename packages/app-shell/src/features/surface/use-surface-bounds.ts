import { useEffect, type RefObject } from "react";
import { usePlatform } from "../../platform";

/**
 * Keeps the native surface aligned with its placeholder element.
 *
 * Geometry can change from panel resizes, window resizes, scrolling ancestors,
 * and the 180 ms panel slide, so every trigger is coalesced through one
 * requestAnimationFrame and re-measured with `getBoundingClientRect`. Nothing
 * is sent while hidden; becoming visible flushes one measurement at once.
 */
export function useSurfaceBounds(
  ref: RefObject<HTMLDivElement | null>,
  instance: number,
  visible: boolean,
): void {
  const { surfaces } = usePlatform();
  useEffect(() => {
    const element = ref.current;
    if (!visible || element === null) return;
    let frame: number | null = null;
    const send = () => {
      frame = null;
      const rect = element.getBoundingClientRect();
      void surfaces
        .setBounds(instance, {
          x: rect.x,
          y: rect.y,
          width: rect.width,
          height: rect.height,
          scale: window.devicePixelRatio,
        })
        .catch(() => undefined);
    };
    const schedule = () => {
      if (frame !== null) return;
      frame = window.requestAnimationFrame(send);
    };
    send();
    const observer = new ResizeObserver(schedule);
    observer.observe(element);
    window.addEventListener("resize", schedule);
    // Capture phase catches scrolling in any ancestor, which moves the placeholder.
    window.addEventListener("scroll", schedule, true);
    document.addEventListener("transitionend", schedule, true);
    return () => {
      if (frame !== null) window.cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
      document.removeEventListener("transitionend", schedule, true);
    };
  }, [instance, ref, surfaces, visible]);
}
