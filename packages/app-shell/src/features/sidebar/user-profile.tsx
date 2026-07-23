import { IconChevronDown, IconLogout, IconSettings } from "@tabler/icons-react";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@ora/ui";
import { useTranslation } from "react-i18next";
import { ColoredAvatar } from "../../components/colored-avatar";
import type { CurrentUser } from "../../lib/types";

interface UserProfileProps {
  user: CurrentUser;
  /** Renders only the avatar - used when the sidebar is collapsed. */
  compact?: boolean;
  onOpenSettings?: () => void;
  onSignOut?: () => void;
}

/**
 * The sidebar footer user chip. Expanded it shows the colored avatar, name,
 * and email; collapsed it shows just the avatar. Both open a small account
 * menu for application settings and sign-out.
 */
export function UserProfile({ user, compact = false, onOpenSettings, onSignOut }: UserProfileProps) {
  const { t } = useTranslation();
  const accountLabel = t("account.label", { name: user.name });
  const trigger = compact ? (
    <Button variant="ghost" size="icon" aria-label={accountLabel} className="rounded-full">
      <ColoredAvatar name={user.name} size="sm" />
    </Button>
  ) : (
    <Button
      variant="ghost"
      size="sm"
      aria-label={accountLabel}
      className="h-auto w-full justify-start gap-2.5 px-2 py-2"
    >
      <ColoredAvatar name={user.name} size="default" />
      <span className="flex min-w-0 flex-1 flex-col text-left">
        <span className="truncate text-[15px] font-semibold text-foreground">{user.name}</span>
        <span className="truncate text-[13px] text-muted-foreground">{user.email}</span>
      </span>
      <IconChevronDown className="size-[18px] shrink-0 text-muted-foreground" />
    </Button>
  );

  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={trigger} />
      <DropdownMenuContent className="w-60" align="start" side="top">
        {onOpenSettings && (
          <>
            <DropdownMenuItem onClick={onOpenSettings}>
              <IconSettings />
              {t("common.settings")}
            </DropdownMenuItem>
            <DropdownMenuSeparator />
          </>
        )}
        <DropdownMenuItem onClick={onSignOut}>
          <IconLogout />
          {t("account.logout")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
