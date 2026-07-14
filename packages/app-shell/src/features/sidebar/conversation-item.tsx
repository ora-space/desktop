import { useEffect, useRef, useState } from "react";
import { DotsVertical, MessageSquare02, Pencil01, Trash01 } from "@untitledui/icons";
import { Button, Dropdown, Input, cx } from "@ora/ui";
import type { Conversation } from "../../lib/types";

interface ConversationItemProps {
  conversation: Conversation;
  active: boolean;
  onSelect: () => void;
  onRename: (title: string) => void;
  onRemove: () => void;
}

/**
 * A single sidebar conversation row: title, active highlight, and a hover
 * affordance to rename or delete. Double-clicking the title enters rename mode.
 */
export function ConversationItem({ conversation, active, onSelect, onRename, onRemove }: ConversationItemProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(conversation.title);
  const inputRef = useRef<HTMLInputElement>(null);

  // Focus and select the title text when entering rename mode.
  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const startEdit = () => {
    setDraft(conversation.title);
    setEditing(true);
  };

  const commit = () => {
    const next = draft.trim();
    if (next && next !== conversation.title) onRename(next);
    else setDraft(conversation.title);
    setEditing(false);
  };

  const cancel = () => {
    setDraft(conversation.title);
    setEditing(false);
  };

  if (editing) {
    return (
      <Input
        ref={inputRef}
        size="sm"
        aria-label="Rename conversation"
        value={draft}
        onChange={setDraft}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commit();
          } else if (event.key === "Escape") {
            event.preventDefault();
            cancel();
          }
        }}
        onBlur={commit}
      />
    );
  }

  return (
    <div className="group relative">
      <button
        type="button"
        onClick={onSelect}
        onDoubleClick={startEdit}
        aria-current={active ? "true" : undefined}
        className={cx(
          "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm font-medium transition duration-100 ease-linear",
          active ? "bg-quaternary text-primary" : "text-secondary hover:bg-primary_hover hover:text-secondary_hover",
        )}
      >
        <MessageSquare02 className="size-[18px] shrink-0 stroke-[1.75px] text-fg-quaternary" />
        <span className="flex-1 truncate">{conversation.title}</span>
      </button>

      <div className={cx("absolute right-1 top-1/2 -translate-y-1/2", !active && "opacity-0 group-hover:opacity-100")}>
        <Dropdown.Root>
          <Button color="tertiary" size="sm" aria-label="Conversation options" noTextPadding className="size-7 p-0">
            <DotsVertical className="size-4 text-fg-quaternary transition-inherit-all group-hover:text-fg-quaternary_hover" />
          </Button>
          <Dropdown.Popover className="w-48">
            <Dropdown.Menu>
              <Dropdown.Item label="Rename" icon={Pencil01} onAction={startEdit} />
              <Dropdown.Item label="Delete" icon={Trash01} onAction={onRemove} />
            </Dropdown.Menu>
          </Dropdown.Popover>
        </Dropdown.Root>
      </div>
    </div>
  );
}
