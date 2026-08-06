import { useRef } from "react";

// This hook compares consecutive render inputs before effects run, so a ref is the
// stable render-local cache that keeps the animation key synchronous with the stream.
/* eslint-disable react-hooks/refs */
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
/* eslint-enable react-hooks/refs */
