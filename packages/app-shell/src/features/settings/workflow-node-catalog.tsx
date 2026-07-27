import {
  IconAdjustmentsAlt,
} from "@tabler/icons-react";
import type { WorkflowNodeKind } from "@ora/workflow-mock";
import { cn } from "@ora/ui";
import { WORKFLOW_NODE_CATALOG } from "./workflow-node-metadata";

/** Offers discoverable click-to-add controls as an accessible alternative to drag-and-drop. */
export function WorkflowNodeCatalog({
  onAdd,
}: {
  onAdd: (kind: WorkflowNodeKind) => void;
}) {
  return (
    <aside className="hidden min-h-0 border-r border-border bg-background lg:flex lg:flex-col">
      <div className="border-b border-border px-4 py-3">
        <div className="flex items-center gap-2">
          <IconAdjustmentsAlt className="size-4 text-muted-foreground" />
          <h3 className="text-xs font-semibold">节点</h3>
        </div>
        <p className="mt-1 text-[11px] leading-4 text-muted-foreground">
          点击添加到画布，再连接执行顺序。
        </p>
      </div>
      <div className="space-y-1.5 overflow-y-auto p-2">
        {WORKFLOW_NODE_CATALOG.map((item) => {
          const Icon = item.icon;
          return (
            <button
              key={item.kind}
              type="button"
              onClick={() => onAdd(item.kind)}
              className="group flex min-h-12 w-full items-center gap-2.5 rounded-lg border border-transparent px-2.5 py-2 text-left outline-none transition-colors hover:border-border hover:bg-muted/65 focus-visible:ring-2 focus-visible:ring-ring"
            >
              <span className={cn("flex size-8 shrink-0 items-center justify-center rounded-md", item.tone)}>
                <Icon className="size-4" stroke={1.8} />
              </span>
              <span className="min-w-0">
                <span className="block text-xs font-medium">{item.label}</span>
                <span className="block truncate text-[10px] text-muted-foreground">{item.description}</span>
              </span>
            </button>
          );
        })}
      </div>
    </aside>
  );
}
