import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { Info } from "lucide-react";

import { cn } from "./utils";

const alertVariants = cva(
  "relative w-full rounded-md border border-border p-4",
  {
    variants: {
      variant: {
        default: "bg-bg text-fg",
        info: "text-fg bg-bg-secondary border-border-subtle",
        destructive: "border-red-500/50 text-red-600 [&_svg]:text-red-600",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

const Alert = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement> & VariantProps<typeof alertVariants>
>(({ className, variant, ...props }, ref) => (
  <div
    ref={ref}
    role="alert"
    className={cn(alertVariants({ variant }), className)}
    {...props}
  />
));
Alert.displayName = "Alert";

const AlertTitle = React.forwardRef<
  HTMLParagraphElement,
  React.HTMLAttributes<HTMLHeadingElement>
>(({ className, children, ...props }, ref) => (
  <h5
    ref={ref}
    className={cn(
      "mb-2 flex items-center gap-2 font-medium leading-none tracking-tight",
      className,
    )}
    {...props}
  >
    <Info className="h-4 w-4 shrink-0" />
    {children}
  </h5>
));
AlertTitle.displayName = "AlertTitle";

const AlertDescription = React.forwardRef<
  HTMLParagraphElement,
  React.HTMLAttributes<HTMLParagraphElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn("text-sm [&_p]:leading-relaxed", className)}
    {...props}
  />
));
AlertDescription.displayName = "AlertDescription";

export { Alert, AlertTitle, AlertDescription };
