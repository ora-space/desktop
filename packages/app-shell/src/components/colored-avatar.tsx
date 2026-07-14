import { Avatar, cx } from "@ora/ui";
import { getAvatarColor, getInitials } from "../lib/avatar";

type AvatarSize = "xs" | "sm" | "md" | "lg" | "xl" | "2xl";

// Mirrors the @ora/ui Avatar's internal initials text sizing so the colored
// initials match the shell's scale when rendered via the `placeholder` slot.
const INITIALS_TEXT: Record<AvatarSize, string> = {
  xs: "text-xs",
  sm: "text-sm",
  md: "text-md",
  lg: "text-lg",
  xl: "text-xl",
  "2xl": "text-display-xs",
};

interface ColoredAvatarProps {
  name: string;
  size?: AvatarSize;
  className?: string;
}

/**
 * A solid-color initials avatar (e.g. "Eric Wang" -> "EW") rendered on top of
 * the @ora/ui Avatar shell. The background hue is picked deterministically
 * from the name, and the initials use a darker shade of the same hue.
 */
export function ColoredAvatar({ name, size = "md", className }: ColoredAvatarProps) {
  const { bg, fg } = getAvatarColor(name);
  return (
    <Avatar
      size={size}
      rounded
      className={className}
      contentClassName={bg}
      placeholder={<span className={cx("font-semibold", INITIALS_TEXT[size], fg)}>{getInitials(name)}</span>}
    />
  );
}
