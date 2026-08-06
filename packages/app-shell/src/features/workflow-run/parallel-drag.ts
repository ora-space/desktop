/**
 * After a horizontal drag on the parallel stage, returns the next index
 * or null when the gesture should snap back without switching.
 */
export function resolveParallelDragSwitch(
  dx: number,
  threshold: number,
  index: number,
  length: number,
): number | null {
  if (length <= 1 || Math.abs(dx) < threshold) {
    return null;
  }
  if (dx < 0 && index < length - 1) {
    return index + 1;
  }
  if (dx > 0 && index > 0) {
    return index - 1;
  }
  return null;
}
