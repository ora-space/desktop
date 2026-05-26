import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "./utils";

const textareaVariants = cva(
  "flex w-full border border-border bg-bg font-sans text-fg placeholder:text-fg-secondary transition-colors focus-visible:outline-none focus-visible:border-primary disabled:cursor-not-allowed disabled:opacity-50 resize-y",
  {
    variants: {
      size: {
        sm: "px-2 py-1 text-xs rounded-sm",
        md: "px-3 py-1.5 text-[13px] rounded-sm",
        lg: "px-3 py-2 text-sm rounded-sm",
      },
    },
    defaultVariants: {
      size: "md",
    },
  },
);

export interface TextareaProps
  extends Omit<React.TextareaHTMLAttributes<HTMLTextAreaElement>, "size">,
    VariantProps<typeof textareaVariants> {}

const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className, size, ...props }, ref) => (
    <textarea
      ref={ref}
      className={cn(textareaVariants({ size, className }))}
      {...props}
    />
  ),
);
Textarea.displayName = "Textarea";

export { Textarea, textareaVariants };
