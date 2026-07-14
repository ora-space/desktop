import type { FC } from "react";
import { Button, cx } from "@ora/ui";

interface IconButtonProps {
  icon: FC<{ className?: string }>;
  /** Accessible label for the button (it has no visible text). */
  label: string;
  onClick?: () => void;
  color?: "tertiary" | "secondary" | "primary";
  className?: string;
  isDisabled?: boolean;
}

/**
 * A compact square icon-only button built on the @ora/ui Button. Used for
 * toolbar actions (new chat, toggle sidebar, copy, etc.) where a labeled
 * button would be too heavy.
 */
export function IconButton({ icon: Icon, label, onClick, color = "tertiary", className, isDisabled }: IconButtonProps) {
  return (
    <Button
      color={color}
      size="sm"
      aria-label={label}
      onClick={onClick}
      isDisabled={isDisabled}
      noTextPadding
      className={cx("size-8 shrink-0 p-0", className)}
    >
      <Icon className="size-[18px] text-fg-quaternary transition-inherit-all group-hover:text-fg-quaternary_hover" />
    </Button>
  );
}
