/** Keeps the file heading slightly below the viewport top. */
export const DIFF_FILE_SCROLL_INSET_PX = 16;

/**
 * Computes the scrollTop that places `element` near the top of `root`.
 * Chat links remount Changes inside a panel that is often still 0-height on
 * the first layout; returning a number in that state would "succeed" at 0 and
 * leave the first file on screen. Callers must retry until this returns a value.
 */
export function diffFileScrollTop(
  root: HTMLElement,
  element: HTMLElement,
): number | null {
  if (root.clientHeight <= 0 || element.offsetHeight <= 0) return null;
  const top =
    element.getBoundingClientRect().top -
    root.getBoundingClientRect().top +
    root.scrollTop -
    DIFF_FILE_SCROLL_INSET_PX;
  if (!Number.isFinite(top)) return null;
  // A later file with a real offsetTop still at viewport-top 0 is unlaid-out.
  if (element.offsetTop > 0 && top <= 0) return null;
  return Math.max(0, top);
}

const DIFF_FILE_ALIGN_TOLERANCE_PX = 64;

/**
 * True when `element` is actually sitting near the top of `root`.
 * Calling scrollTo is not enough: virtualized placeholders above the file can
 * shrink after the first jump and leave the requested section off-screen.
 */
export function isDiffFileAligned(
  root: HTMLElement,
  element: HTMLElement,
): boolean {
  if (root.clientHeight <= 0 || element.offsetHeight <= 0) return false;
  const offset =
    element.getBoundingClientRect().top - root.getBoundingClientRect().top;
  if (!Number.isFinite(offset)) return false;
  return (
    Math.abs(offset - DIFF_FILE_SCROLL_INSET_PX) <= DIFF_FILE_ALIGN_TOLERANCE_PX
  );
}

/**
 * True when the requested file is at the viewport top and files above it have
 * finished replacing estimated placeholders. Aligning against placeholders
 * overshoots once those sections take their real height.
 */
export function isDiffFileScrollSettled(
  root: HTMLElement,
  element: HTMLElement,
): boolean {
  if (!isDiffFileAligned(root, element)) return false;
  let sibling = element.previousElementSibling;
  while (sibling instanceof HTMLElement) {
    if (sibling.querySelector('[aria-busy="true"]')) return false;
    sibling = sibling.previousElementSibling;
  }
  return true;
}
