/**
 * Finds the nearest ancestor that can scroll vertically past its client box.
 */
export function findVerticalScrollParent(from: HTMLElement): HTMLElement | null {
  let node: HTMLElement | null = from.parentElement;
  while (node !== null) {
    const { overflowY } = getComputedStyle(node);
    const scrollableOverflow =
      overflowY === "auto"
      || overflowY === "scroll"
      || overflowY === "overlay";
    if (scrollableOverflow && node.scrollHeight > node.clientHeight + 1) {
      return node;
    }
    node = node.parentElement;
  }
  return null;
}

/**
 * When a nested scrollport is at its edge (or not scrollable), forward the
 * leftover wheel delta to the parent stage scroll so readers do not have to
 * move the cursor out of the conversation to reach HITL / metrics below.
 */
export function chainWheelToScrollParent(
  event: { deltaY: number; preventDefault: () => void },
  element: HTMLElement,
): void {
  const { deltaY } = event;
  if (deltaY === 0) {
    return;
  }

  const maxScroll = element.scrollHeight - element.clientHeight;
  const scrollingDown = deltaY > 0;
  const atTop = element.scrollTop <= 0;
  const atBottom = maxScroll <= 0 || element.scrollTop >= maxScroll - 1;
  const shouldChain =
    maxScroll <= 0
    || (scrollingDown && atBottom)
    || (!scrollingDown && atTop);
  if (!shouldChain) {
    return;
  }

  const parent = findVerticalScrollParent(element);
  if (parent === null) {
    return;
  }

  const parentMax = parent.scrollHeight - parent.clientHeight;
  const nextTop = Math.min(parentMax, Math.max(0, parent.scrollTop + deltaY));
  if (nextTop === parent.scrollTop) {
    return;
  }

  event.preventDefault();
  parent.scrollTop = nextTop;
}
