import { Avatar, AvatarFallback, AvatarImage, cn } from "@ora/ui";
import { IconMessageCircle } from "@tabler/icons-react";

/** Served from the web app's static asset directory so it works in dev and the packaged build. */
const LOGO_SRC = "/ora-logo.svg";

type AvatarSize = "sm" | "default" | "lg";

const ICON_SIZE: Record<AvatarSize, string> = {
  sm: "size-3.5",
  default: "size-4",
  lg: "size-5",
};

interface OraMarkProps {
  size?: AvatarSize;
  className?: string;
}

/** The Ora brand mark: the product logo, falling back to a primary badge glyph if the asset is unavailable. */
export function OraMark({ size = "default", className }: OraMarkProps) {
  return (
    <Avatar size={size} className={cn("overflow-hidden rounded-lg", className)}>
      <AvatarImage
        src={LOGO_SRC}
        alt="Ora"
        className="rounded-lg object-cover"
      />
      <AvatarFallback className="rounded-lg bg-primary text-primary-foreground">
        <IconMessageCircle className={ICON_SIZE[size]} />
      </AvatarFallback>
    </Avatar>
  );
}
