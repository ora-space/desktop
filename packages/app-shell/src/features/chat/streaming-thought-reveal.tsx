import { useLayoutEffect, useRef, type ReactNode } from "react";

/** Tracks the appended suffix without replaying reveal motion for stable streamed content. */
export function useStreamingThoughtRevealStart(content: string, streaming: boolean) {
  const previousRef = useRef<{
    contentLength: number;
    revealStart: number;
    streaming: boolean;
  } | null>(null);
  const previous = previousRef.current;
  if (previous?.contentLength === content.length && previous.streaming === streaming) {
    return previous.revealStart;
  }
  // ChatThought chunks are append-only in the conversation store. Tracking lengths keeps each
  // streamed update O(1) instead of repeatedly scanning the complete accumulated thought.
  const revealStart = streaming
    ? previous !== null && previous.streaming && content.length >= previous.contentLength
      ? previous.contentLength
      : 0
    : content.length;
  previousRef.current = { contentLength: content.length, revealStart, streaming };
  return revealStart;
}

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
