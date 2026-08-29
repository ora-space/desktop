import { IconChevronDown } from "@tabler/icons-react";
import { Button } from "@ora/ui";
import { useTranslation } from "react-i18next";
import { InitialsAvatar } from "../../components/initials-avatar";
import type { CurrentUser } from "../../lib/types";

interface UserProfileProps {
  user: CurrentUser;
  /** Renders only the avatar - used when the sidebar is collapsed. */
  compact?: boolean;
  /** Opens the settings dialog; account sign-out is not implemented yet. */
  onOpenSettings: () => void;
}

/**
 * The sidebar footer user chip. Clicking it opens the settings dialog. Expanded
 * it shows the colored avatar, name, and email; collapsed it shows the avatar.
 */
export function UserProfile({
  user,
  compact = false,
  onOpenSettings,
}: UserProfileProps) {
  const { t } = useTranslation();
  const accountLabel = t("account.label", { name: user.name });

  if (compact) {
    return (
      <Button
        variant="ghost"
        size="icon"
        aria-label={accountLabel}
        onClick={onOpenSettings}
        className="rounded-full"
      >
        <InitialsAvatar name={user.name} size="sm" />
      </Button>
    );
  }

  return (
    <Button
      variant="ghost"
      size="sm"
      aria-label={accountLabel}
      onClick={onOpenSettings}
      className="h-auto w-full justify-start gap-2.5 px-2 py-2"
    >
      <InitialsAvatar name={user.name} size="default" />
      <span className="flex min-w-0 flex-1 flex-col text-left">
        <span className="truncate text-[15px] font-semibold text-foreground">
          {user.name}
        </span>
        {/* Always render the second row so the profile keeps its two-line layout even
            when no email is configured; a non-breaking space preserves the line box. */}
        <span className="truncate text-[13px] text-muted-foreground">
          {user.email || "\u00a0"}
        </span>
      </span>
      <IconChevronDown className="size-[18px] shrink-0 text-muted-foreground" />
    </Button>
  );
}
