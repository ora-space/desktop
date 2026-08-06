import { useLayoutEffect, useRef, type ReactNode } from "react";

/** Applies a restrained opacity-only reveal to the latest streamed thought suffix. */
export function StreamingThoughtReveal({ children }: { children: ReactNode }) {
  const spanRef = useRef<HTMLSpanElement>(null);

  useLayoutEffect(() => {
    const span = spanRef.current;
    if (
      span === null
      || typeof span.animate !== "function"
      || window.matchMedia("(prefers-reduced-motion: reduce)").matches
    ) return;
    const animation = span.animate(
      [{ opacity: 0.55 }, { opacity: 1 }],
      { duration: 140, easing: "cubic-bezier(0.2, 0, 0, 1)" },
    );
    animation.addEventListener("finish", () => animation.cancel(), { once: true });
    return () => animation.cancel();
  }, []);

  return (
    <span ref={spanRef} data-stream-thought-reveal>
      {children}
    </span>
  );
}
