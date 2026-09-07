import React, {
  useEffect,
  useState,
  useRef,
  useCallback,
  useLayoutEffect,
} from "react";
import { useCurrentEditor, Editor } from "@tiptap/react";
import { TextSelection } from "@tiptap/pm/state";
import type { TOCItem } from "./toc-extension.ts";
import { getTOCItems } from "./toc-extension.ts";
import "./toc.scss";

const DEFAULT_TOC_CLASS =
  "tiptap-toc scrollbar-hide fixed top-[20vh] right-[1rem] z-[1000] w-[250px] max-h-[60vh] overflow-y-auto overflow-x-hidden hidden xl:block";

interface TOCProps {
  editor?: Editor | null;
  scrollRef?: React.RefObject<HTMLDivElement | null>;
  scrollContainerId?: string;
  dependency?: unknown;
  className?: string;
}

export const TableOfContents: React.FC<TOCProps> = ({
  editor: propEditor,
  scrollRef,
  scrollContainerId,
  dependency,
  className,
}) => {
  const { editor: currentEditor } = useCurrentEditor() || {};
  const editor = propEditor || currentEditor;

  const [items, setItems] = useState<TOCItem[]>([]);
  const [activeId, setActiveId] = useState<string>("");
  const [loadKey, setLoadKey] = useState({ editor, dependency });
  if (loadKey.editor !== editor || loadKey.dependency !== dependency) {
    setLoadKey({ editor, dependency });
    setItems([]);
    setActiveId("");
  }
  const activeIdRef = useRef(activeId);
  const [isHovered, setIsHovered] = useState(false);
  const isHoveredRef = useRef(isHovered);
  const tocRef = useRef<HTMLDivElement>(null);
  // Track scroll behaviour option from TOC extension
  const scrollBehavior: ScrollBehavior = "smooth";

  useEffect(() => {
    isHoveredRef.current = isHovered;
  }, [isHovered]);
  useEffect(() => {
    activeIdRef.current = activeId;
  }, [activeId]);

  // ---- Item Loading ----
  // Pull headings from ProseMirror state.doc via the plugin state (source of truth).
  // Listen to both `transaction` and `update` events so we capture content loads
  // from ProseMirrorState.create (IndexedDB) and normal edits.
  useLayoutEffect(() => {
    if (!editor) return;

    let cancelled = false;
    let retryTimer: ReturnType<typeof setTimeout>;
    let retryCount = 0;
    const MAX_RETRIES = 20; // up to 2 seconds

    const readItems = () => {
      if (cancelled) return;
      const newItems = getTOCItems(editor);
      if (newItems.length > 0) {
        setItems(newItems);
      } else if (retryCount < MAX_RETRIES) {
        retryCount++;
        retryTimer = setTimeout(readItems, 100);
      }
    };

    // Initial read after a microtask so updateState has been applied
    retryTimer = setTimeout(readItems, 50);

    const handleUpdate = () => {
      if (cancelled) return;
      const newItems = getTOCItems(editor);
      setItems(newItems);
    };

    editor.on("transaction", handleUpdate);
    editor.on("update", handleUpdate);

    return () => {
      cancelled = true;
      clearTimeout(retryTimer);
      editor.off("transaction", handleUpdate);
      editor.off("update", handleUpdate);
    };
  }, [editor, dependency]);

  // Resolve the scroll container element
  const getScrollContainer = useCallback(() => {
    if (scrollRef?.current) return scrollRef.current;
    if (scrollContainerId)
      return document.getElementById(scrollContainerId) as HTMLElement | null;
    return null;
  }, [scrollRef, scrollContainerId]);

  // Resolve the editor DOM root for scoped queries
  const getEditorRoot = useCallback((): HTMLElement | null => {
    if (!editor) return null;
    return editor.view.dom as HTMLElement;
  }, [editor]);

  // ---- Active Heading Tracking via IntersectionObserver ----
  useEffect(() => {
    const container = getScrollContainer();
    const editorRoot = getEditorRoot();
    if (!container || !editorRoot || !items.length) return;

    // Collect current heading elements scoped to editor.view.dom
    const headingElements =
      editorRoot.querySelectorAll<HTMLElement>("[data-toc-id]");
    if (!headingElements.length) return;

    // Map element -> tocId for quick lookup
    const elToId = new Map<Element, string>();
    headingElements.forEach((el) => {
      const id = el.getAttribute("data-toc-id");
      if (id) elToId.set(el, id);
    });

    // We maintain a set of currently-intersecting heading IDs and pick the
    // topmost visible one (by document order) as active.
    const visibleSet = new Set<string>();

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const id = elToId.get(entry.target);
          if (!id) continue;
          if (entry.isIntersecting) {
            visibleSet.add(id);
          } else {
            visibleSet.delete(id);
          }
        }

        // Determine active: the *last* item (in doc order) whose top is
        // inside the active zone (top 20% of the viewport).  Falls back to
        // the first visible heading.
        let bestId = "";
        for (const item of items) {
          if (visibleSet.has(item.id)) {
            bestId = item.id;
          }
        }

        // If nothing is in the active zone, find the heading that has scrolled
        // past the top of the viewport most recently (last heading above viewport top).
        if (!bestId) {
          const containerRect = container.getBoundingClientRect();
          for (let i = items.length - 1; i >= 0; i--) {
            const el = editorRoot.querySelector<HTMLElement>(
              `[data-toc-id="${items[i].id}"]`,
            );
            if (el) {
              const elRect = el.getBoundingClientRect();
              if (
                elRect.top <=
                containerRect.top + container.clientHeight * 0.2
              ) {
                bestId = items[i].id;
                break;
              }
            }
          }
        }

        if (bestId !== activeIdRef.current && !isHoveredRef.current) {
          setActiveId(bestId);
          // Scroll the TOC sidebar itself to keep active item visible
          const tocItem = document.getElementById(`toc-item-${bestId}`);
          if (tocItem && tocRef.current) {
            const tocRect = tocRef.current.getBoundingClientRect();
            const itemRect = tocItem.getBoundingClientRect();
            const offsetTop =
              itemRect.top -
              tocRect.top +
              tocRef.current.scrollTop -
              tocRef.current.clientHeight / 2;
            tocRef.current.scrollTo({
              top: Math.max(0, offsetTop),
              behavior: scrollBehavior,
            });
          }
        }
      },
      {
        root: container,
        // Active zone = top 20% of the container
        rootMargin: "0px 0px -80% 0px",
        threshold: 0,
      },
    );

    headingElements.forEach((el) => observer.observe(el));

    return () => {
      observer.disconnect();
    };
  }, [items, getScrollContainer, getEditorRoot, scrollBehavior]);

  // ---- Fallback: scroll listener for edge-cases IntersectionObserver misses ----
  // (e.g. when the user is at the very top or bottom of the document)
  useEffect(() => {
    const container = getScrollContainer();
    const editorRoot = getEditorRoot();
    if (!container || !editorRoot || !items.length) return;

    let rafId: number;
    const handleScroll = () => {
      cancelAnimationFrame(rafId);
      rafId = requestAnimationFrame(() => {
        if (isHoveredRef.current) return;

        const containerRect = container.getBoundingClientRect();
        const threshold = containerRect.top + container.clientHeight * 0.2;

        // Scope query to editor root
        const headings =
          editorRoot.querySelectorAll<HTMLElement>("[data-toc-id]");

        const visibleHeadings = Array.from(headings).filter((heading) => {
          const rect = heading.getBoundingClientRect();
          return rect.top <= threshold;
        });

        if (visibleHeadings.length > 0) {
          const lastVisible = visibleHeadings[visibleHeadings.length - 1];
          const tocId = lastVisible.getAttribute("data-toc-id");
          if (tocId && tocId !== activeIdRef.current) {
            setActiveId(tocId);
            const tocItem = document.getElementById(`toc-item-${tocId}`);
            if (tocItem && tocRef.current) {
              const tocRect = tocRef.current.getBoundingClientRect();
              const itemRect = tocItem.getBoundingClientRect();
              const offsetTop =
                itemRect.top -
                tocRect.top +
                tocRef.current.scrollTop -
                tocRef.current.clientHeight / 2;
              tocRef.current.scrollTo({
                top: Math.max(0, offsetTop),
                behavior: scrollBehavior,
              });
            }
          }
        } else {
          if (activeIdRef.current !== "") {
            setActiveId("");
            tocRef.current?.scrollTo({ top: 0, behavior: scrollBehavior });
          }
        }
      });
    };

    container.addEventListener("scroll", handleScroll, { passive: true });
    // Run once to set initial active heading
    handleScroll();

    return () => {
      cancelAnimationFrame(rafId);
      container.removeEventListener("scroll", handleScroll);
    };
  }, [items, getScrollContainer, getEditorRoot, scrollBehavior]);

  const scrollToHeading = (item: TOCItem) => {
    if (!editor) return;

    const container = getScrollContainer();
    const editorRoot = getEditorRoot();
    if (!container || !editorRoot) return;

    // Scope query to editor root
    const element = editorRoot.querySelector<HTMLElement>(
      `[data-toc-id="${item.id}"]`,
    );
    if (!element) return;

    const rect = element.getBoundingClientRect();
    const containerRect = container.getBoundingClientRect();
    const offset = rect.top - containerRect.top;
    // add small offset to handle toc active item highlight issue, i.e. 10
    const targetY =
      container.scrollTop + offset - container.clientHeight * 0.2 + 10;

    container.scrollTo({ top: targetY, behavior: scrollBehavior });

    setTimeout(() => {
      const { tr } = editor.state;
      const headingNode = tr.doc.nodeAt(item.pos);
      if (headingNode) {
        const endPos = item.pos + headingNode.nodeSize - 1;
        const resolvedPos = tr.doc.resolve(
          Math.min(endPos, tr.doc.content.size),
        );
        const selection = TextSelection.create(tr.doc, resolvedPos.pos);
        editor.view.dispatch(tr.setSelection(selection));
        if (editor.isEditable) {
          editor.view.focus();
        }
      }
    }, 300);
  };

  const getItemOpacity = (item: TOCItem) => {
    if (isHovered) return 1;
    // When no heading has scrolled past the threshold yet, show all items
    if (!activeId) return 0.6;
    // If this is the active item, full opacity
    if (item.id === activeId) return 1;

    // Check if this item is a parent of the active item
    const activeItem = items.find((i) => i.id === activeId);
    if (!activeItem) return 0;

    // If this item is a parent (lower level number) and comes before the active item
    if (item.level < activeItem.level && item.pos < activeItem.pos) {
      // Check if there's no other heading of the same or lower level between this and active
      const itemsBetween = items.filter(
        (i) =>
          i.pos > item.pos && i.pos < activeItem.pos && i.level <= item.level,
      );
      if (itemsBetween.length === 0) return 1;
    }

    return 0;
  };

  if (!items.length || !editor) return null;

  return (
    <div
      ref={tocRef}
      className={className ?? DEFAULT_TOC_CLASS}
      onMouseEnter={() => setIsHovered(true)}
      onMouseLeave={() => setIsHovered(false)}
    >
      {items.map((item) => (
        <div
          key={item.id}
          className={`tiptap-toc-item tiptap-toc-level-${item.level} ${
            item.id === activeId ? "tiptap-toc-active" : ""
          }`}
        >
          <div className="toc-item-circle"></div>
          <div
            id={`toc-item-${item.id}`}
            className="toc-item-title"
            style={{ opacity: getItemOpacity(item) }}
            onClick={() => scrollToHeading(item)}
          >
            {item.text}
          </div>
        </div>
      ))}
    </div>
  );
};
