import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "./utils";

const badgeVariants = cva(
  "inline-flex items-center rounded-full border border-border px-2.5 py-0.5 text-xs font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-primary focus:ring-offset-1",
  {
    variants: {
      variant: {
        default: "border-transparent bg-btn-bg text-btn-fg",
        secondary: "border-transparent bg-bg-secondary text-fg",
        destructive: "border-transparent bg-red-600 text-white",
        outline: "text-fg",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

export interface BadgeProps
  extends
    React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return (
    <div className={cn(badgeVariants({ variant }), className)} {...props} />
  );
}

export { Badge, badgeVariants };
