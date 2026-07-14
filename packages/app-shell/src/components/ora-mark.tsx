import { Avatar, cx } from "@ora/ui";
import { AnnotationDots } from "@untitledui/icons";

type AvatarSize = "xs" | "sm" | "md" | "lg" | "xl" | "2xl";

const ICON_SIZE: Record<AvatarSize, string> = {
  xs: "size-4",
  sm: "size-5",
  md: "size-6",
  lg: "size-7",
  xl: "size-8",
  "2xl": "size-8",
};

interface OraMarkProps {
  size?: AvatarSize;
  className?: string;
}

/** The Ora brand mark: a brand-colored rounded square with a chat glyph. */
export function OraMark({ size = "md", className }: OraMarkProps) {
  return (
    <Avatar
      size={size}
      rounded={false}
      className={className}
      contentClassName="bg-brand-solid"
      placeholder={<AnnotationDots className={cx("text-white", ICON_SIZE[size])} />}
    />
  );
}
