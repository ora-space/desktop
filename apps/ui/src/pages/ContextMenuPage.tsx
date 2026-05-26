import { useState } from "react";
import {
  ContextMenu,
  ContextMenuCheckboxItem,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuRadioGroup,
  ContextMenuRadioItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@ora/ui";
import { Section, Row } from "./shared";

function TriggerBox({ label }: { label: string }) {
  return (
    <div className="flex h-20 w-48 items-center justify-center rounded-md border border-dashed border-border bg-bg-subtle text-xs text-fg-secondary select-none">
      {label}
    </div>
  );
}

export default function ContextMenuPage() {
  const [showStatusBar, setShowStatusBar] = useState(true);
  const [showPanel, setShowPanel] = useState(false);
  const [radioValue, setRadioValue] = useState("pedro");

  return (
    <>
      <Section title="Context Menu">
        <Row label="basic">
          <ContextMenu>
            <ContextMenuTrigger>
              <TriggerBox label="Right-click here" />
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem>
                Back
                <ContextMenuShortcut>⌘[</ContextMenuShortcut>
              </ContextMenuItem>
              <ContextMenuItem>
                Forward
                <ContextMenuShortcut>⌘]</ContextMenuShortcut>
              </ContextMenuItem>
              <ContextMenuItem>
                Reload
                <ContextMenuShortcut>⌘R</ContextMenuShortcut>
              </ContextMenuItem>
              <ContextMenuSub>
                <ContextMenuSubTrigger>More Tools</ContextMenuSubTrigger>
                <ContextMenuSubContent>
                  <ContextMenuItem>
                    Save Page As…
                    <ContextMenuShortcut>⌘S</ContextMenuShortcut>
                  </ContextMenuItem>
                  <ContextMenuItem>Create Shortcut…</ContextMenuItem>
                  <ContextMenuSeparator />
                  <ContextMenuItem>Developer Tools</ContextMenuItem>
                </ContextMenuSubContent>
              </ContextMenuSub>
              <ContextMenuSeparator />
              <ContextMenuCheckboxItem
                checked={showStatusBar}
                onCheckedChange={setShowStatusBar}
              >
                Status Bar
              </ContextMenuCheckboxItem>
              <ContextMenuCheckboxItem
                checked={showPanel}
                onCheckedChange={setShowPanel}
              >
                Activity Bar
              </ContextMenuCheckboxItem>
              <ContextMenuSeparator />
              <ContextMenuLabel>Team</ContextMenuLabel>
              <ContextMenuRadioGroup value={radioValue} onValueChange={setRadioValue}>
                <ContextMenuRadioItem value="pedro">Pedro Duarte</ContextMenuRadioItem>
                <ContextMenuRadioItem value="colm">Colm Tuite</ContextMenuRadioItem>
              </ContextMenuRadioGroup>
            </ContextMenuContent>
          </ContextMenu>
        </Row>

        <Row label="disabled items">
          <ContextMenu>
            <ContextMenuTrigger>
              <TriggerBox label="Right-click here" />
            </ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuItem>Cut</ContextMenuItem>
              <ContextMenuItem>Copy</ContextMenuItem>
              <ContextMenuItem disabled>Paste</ContextMenuItem>
              <ContextMenuSeparator />
              <ContextMenuItem disabled>Delete</ContextMenuItem>
            </ContextMenuContent>
          </ContextMenu>
        </Row>
      </Section>
    </>
  );
}
