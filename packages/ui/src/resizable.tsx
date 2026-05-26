import * as React from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { GripHorizontal, GripVertical } from "lucide-react";
import { cn } from "./utils";

const ResizablePanelGroup = ({
  className,
  ...props
}: React.ComponentProps<typeof Group>) => (
  <Group className={cn("h-full w-full", className)} {...props} />
);
ResizablePanelGroup.displayName = "ResizablePanelGroup";

const ResizablePanel = Panel;

type ResizableHandleProps = React.ComponentProps<typeof Separator> & {
  withHandle?: boolean;
  orientation?: "horizontal" | "vertical";
};

const ResizableHandle = ({
  withHandle,
  orientation = "horizontal",
  className,
  ...props
}: ResizableHandleProps) => (
  <Separator
    className={cn(
      "relative flex items-center justify-center bg-border transition-colors hover:bg-primary/40 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary focus-visible:ring-offset-1",
      orientation === "vertical"
        ? "h-px w-full after:absolute after:inset-x-0 after:top-1/2 after:h-1 after:-translate-y-1/2"
        : "w-px after:absolute after:inset-y-0 after:left-1/2 after:w-1 after:-translate-x-1/2",
      className,
    )}
    {...props}
  >
    {withHandle && (
      <div className={cn(
        "z-10 flex items-center justify-center rounded-sm border border-border bg-border",
        orientation === "vertical" ? "h-3 w-4" : "h-4 w-3",
      )}>
        {orientation === "vertical"
          ? <GripHorizontal className="h-2.5 w-2.5 text-fg-secondary" />
          : <GripVertical className="h-2.5 w-2.5 text-fg-secondary" />}
      </div>
    )}
  </Separator>
);
ResizableHandle.displayName = "ResizableHandle";

export { ResizablePanelGroup, ResizablePanel, ResizableHandle };
