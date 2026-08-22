import { Extension } from "@tiptap/core";
import type { Node as PmNode } from "@tiptap/pm/model";
import {
  NodeSelection,
  Plugin,
  PluginKey,
  TextSelection,
} from "@tiptap/pm/state";
import type { EditorView } from "@tiptap/pm/view";

const CHIP_NODE_TYPES = new Set(["composerFile", "promptToken"]);
const composerChipSelectionKey = new PluginKey("composerChipSelection");
const CHIP_SELECTED_ATTR = "data-chip-selected";

/**
 * Builds a TextSelection for a pointer drag. Atom chips are `user-select: none`
 * and as wide as a filename, so caret mapping lands on whichever half of the
 * chip the pointer is over — native selection would wait until the midpoint.
 * Once the pointer is inside a chip, snap the head (and the anchor, if the
 * press started on that same chip) so the whole atom is in the range.
 */
export function textSelectionForChipDrag(
  doc: PmNode,
  anchorPos: number,
  headPos: number,
  pointerInside: number,
): TextSelection {
  let anchor = anchorPos;
  let head = headPos;
  const chip = chipRangeAt(doc, pointerInside);
  if (chip !== null) {
    if (headPos >= anchorPos) {
      head = Math.max(headPos, chip.end);
      if (anchorPos > chip.start && anchorPos < chip.end) anchor = chip.start;
    } else {
      head = Math.min(headPos, chip.start);
      if (anchorPos > chip.start && anchorPos < chip.end) anchor = chip.end;
    }
  }
  return TextSelection.create(doc, anchor, head);
}

/** Inclusive chip span when `pos` is the atom's own position. */
export function chipRangeAt(
  doc: PmNode,
  pos: number,
): { start: number; end: number } | null {
  if (pos < 0 || pos > doc.content.size) return null;
  const node = doc.nodeAt(pos);
  if (node === null || !CHIP_NODE_TYPES.has(node.type.name)) return null;
  return { start: pos, end: pos + node.nodeSize };
}

/** Selects exactly one chip atom so ctrl/double-clicks cannot span siblings. */
export function pinComposerChipSelection(
  view: Pick<EditorView, "dispatch" | "state">,
  nodePos: number,
  event: Pick<MouseEvent, "preventDefault">,
): boolean {
  view.dispatch(
    view.state.tr.setSelection(NodeSelection.create(view.state.doc, nodePos)),
  );
  event.preventDefault();
  return true;
}

/**
 * File and prompt chips are atoms: native `::selection` skips them, and a
 * React node-view decoration class would re-render on every mousemove. This
 * plugin snaps the dragged TextSelection onto any chip under the pointer and
 * paints `data-chip-selected` imperatively so highlight tracks the mouse.
 */
export const ComposerChipSelection = Extension.create({
  name: "composerChipSelection",

  addProseMirrorPlugins() {
    /** Document position where the current button-1 drag started. */
    let dragAnchor: number | null = null;
    let lastPointer: { left: number; top: number } | null = null;

    const clearDrag = () => {
      dragAnchor = null;
      lastPointer = null;
    };

    return [
      new Plugin({
        key: composerChipSelectionKey,
        view(editorView) {
          paintChipSelection(editorView);
          const onUp = () => clearDrag();
          window.addEventListener("mouseup", onUp, true);
          window.addEventListener("blur", onUp);
          return {
            update(nextView) {
              paintChipSelection(nextView);
            },
            destroy() {
              window.removeEventListener("mouseup", onUp, true);
              window.removeEventListener("blur", onUp);
              clearDrag();
            },
          };
        },
        props: {
          createSelectionBetween(view) {
            if (dragAnchor === null || lastPointer === null) return null;
            const hit = view.posAtCoords(lastPointer);
            if (
              hit === null ||
              chipRangeAt(view.state.doc, hit.inside) === null
            ) {
              return null;
            }
            return textSelectionForChipDrag(
              view.state.doc,
              dragAnchor,
              hit.pos,
              hit.inside,
            );
          },
          handleClickOn(view, _pos, node, nodePos, event) {
            if (!CHIP_NODE_TYPES.has(node.type.name)) return false;
            if (event.ctrlKey || event.metaKey) {
              return pinComposerChipSelection(view, nodePos, event);
            }
            return false;
          },
          handleDoubleClickOn(view, _pos, node, nodePos, event) {
            if (!CHIP_NODE_TYPES.has(node.type.name)) return false;
            return pinComposerChipSelection(view, nodePos, event);
          },
          handleDOMEvents: {
            mousedown(view, event) {
              if (event.button !== 0) return false;
              lastPointer = { left: event.clientX, top: event.clientY };
              if (event.shiftKey) {
                dragAnchor = view.state.selection.anchor;
                return false;
              }
              const hit = view.posAtCoords(lastPointer);
              dragAnchor = hit?.pos ?? null;
              return false;
            },
            mousemove(view, event) {
              if (event.buttons !== 1 || dragAnchor === null) return false;
              lastPointer = { left: event.clientX, top: event.clientY };
              const hit = view.posAtCoords(lastPointer);
              if (hit === null) return false;
              const overChip = chipRangeAt(view.state.doc, hit.inside) !== null;
              if (!overChip) return false;
              const next = textSelectionForChipDrag(
                view.state.doc,
                dragAnchor,
                hit.pos,
                hit.inside,
              );
              if (!next.eq(view.state.selection)) {
                view.dispatch(view.state.tr.setSelection(next));
              }
              // Claim the event so native selection cannot jump over the atom.
              return true;
            },
            /** Browser dblclick on adjacent user-select:none atoms selects them all. */
            dblclick(view, event) {
              const coords = view.posAtCoords({
                left: event.clientX,
                top: event.clientY,
              });
              if (coords === null) return false;
              const directPos = coords.inside >= 0 ? coords.inside : coords.pos;
              const direct = view.state.doc.nodeAt(directPos);
              if (direct !== null && CHIP_NODE_TYPES.has(direct.type.name)) {
                return pinComposerChipSelection(view, directPos, event);
              }
              const $pos = view.state.doc.resolve(coords.pos);
              if (
                $pos.nodeAfter !== null &&
                CHIP_NODE_TYPES.has($pos.nodeAfter.type.name)
              ) {
                return pinComposerChipSelection(view, $pos.pos, event);
              }
              if (
                $pos.nodeBefore !== null &&
                CHIP_NODE_TYPES.has($pos.nodeBefore.type.name)
              ) {
                return pinComposerChipSelection(
                  view,
                  $pos.pos - $pos.nodeBefore.nodeSize,
                  event,
                );
              }
              return false;
            },
          },
        },
      }),
    ];
  },
});

/**
 * Paints range-selected chips on the live DOM. Skips NodeSelection so the
 * existing `ProseMirror-selectednode` ring stays the click-to-focus treatment.
 */
function paintChipSelection(view: EditorView): void {
  const root = view.dom;
  const { selection, doc } = view.state;
  const keep = new Set<Element>();
  if (!selection.empty && !(selection instanceof NodeSelection)) {
    doc.nodesBetween(selection.from, selection.to, (node, pos) => {
      if (!CHIP_NODE_TYPES.has(node.type.name)) return;
      const dom = view.nodeDOM(pos);
      const el = elementForChipDom(dom);
      if (el === null) return;
      el.setAttribute(CHIP_SELECTED_ATTR, "true");
      keep.add(el);
    });
  }
  root.querySelectorAll(`[${CHIP_SELECTED_ATTR}]`).forEach((el) => {
    if (!keep.has(el)) el.removeAttribute(CHIP_SELECTED_ATTR);
  });
}

function elementForChipDom(dom: Node | null | undefined): HTMLElement | null {
  if (dom instanceof HTMLElement) return dom;
  if (dom?.parentElement instanceof HTMLElement) return dom.parentElement;
  return null;
}
