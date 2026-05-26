import * as React from "react";
import { cn } from "./utils";

interface EmptyProps extends React.HTMLAttributes<HTMLDivElement> {
  icon?: React.ReactNode;
  title?: string;
  description?: string;
  action?: React.ReactNode;
}

const Empty = React.forwardRef<HTMLDivElement, EmptyProps>(
  ({ className, icon, title, description, action, children, ...props }, ref) => (
    <div
      ref={ref}
      className={cn(
        "flex flex-col items-center justify-center gap-3 py-12 text-center",
        className,
      )}
      {...props}
    >
      {icon && (
        <div className="flex h-10 w-10 items-center justify-center rounded-lg border border-border bg-bg-subtle text-fg-secondary">
          {icon}
        </div>
      )}
      {title && <p className="text-sm font-medium text-fg">{title}</p>}
      {description && (
        <p className="max-w-xs text-xs text-fg-secondary">{description}</p>
      )}
      {action}
      {children}
    </div>
  ),
);
Empty.displayName = "Empty";

export { Empty };
export type { EmptyProps };
