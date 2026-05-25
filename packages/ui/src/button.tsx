import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "./utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 whitespace-nowrap font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-1 disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        primary:
          "bg-btn-bg text-btn-fg hover:bg-primary",
        secondary:
          "bg-bg-subtle text-fg border border-border hover:bg-bg-secondary",
        ghost:
          "text-fg hover:bg-border-subtle",
        destructive:
          "bg-red-600 text-white hover:bg-red-700",
        outline:
          "border border-border bg-transparent text-fg hover:bg-bg-subtle",
      },
      size: {
        sm: "h-7 px-2.5 text-xs rounded-sm",
        md: "h-8 px-3 text-[13px] rounded-sm",
        lg: "h-9 px-4 text-sm rounded-md",
        icon: "h-8 w-8 rounded-sm",
      },
    },
    defaultVariants: {
      variant: "primary",
      size: "md",
    },
  }
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        className={cn(buttonVariants({ variant, size, className }))}
        ref={ref}
        {...props}
      />
    );
  }
);
Button.displayName = "Button";

export { Button, buttonVariants };
