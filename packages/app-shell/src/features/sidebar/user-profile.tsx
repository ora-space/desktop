import { ChevronDown, Settings04, UserLeft01 } from "@untitledui/icons";
import { Button, Dropdown } from "@ora/ui";
import { ColoredAvatar } from "../../components/colored-avatar";
import type { CurrentUser } from "../../lib/types";

interface UserProfileProps {
  user: CurrentUser;
  /** Renders only the avatar — used when the sidebar is collapsed. */
  compact?: boolean;
  onSignOut?: () => void;
}

/**
 * The sidebar footer user chip. Expanded it shows the colored avatar, name,
 * and email; collapsed it shows just the avatar. Both open a small account
 * menu (Settings / Log out).
 */
export function UserProfile({ user, compact = false, onSignOut }: UserProfileProps) {
  const trigger = compact ? (
    <Button color="tertiary" size="sm" aria-label={`${user.name} account`} noTextPadding className="size-9 p-0 rounded-full">
      <ColoredAvatar name={user.name} size="sm" />
    </Button>
  ) : (
    <Button color="tertiary" size="sm" aria-label={`${user.name} account`} noTextPadding className="w-full justify-start gap-2 p-1.5">
      <ColoredAvatar name={user.name} size="sm" />
      <span className="flex min-w-0 flex-1 flex-col text-left">
        <span className="truncate text-sm font-semibold text-primary">{user.name}</span>
        <span className="truncate text-xs text-tertiary">{user.email}</span>
      </span>
      <ChevronDown className="size-4 shrink-0 text-fg-quaternary" />
    </Button>
  );

  return (
    <Dropdown.Root>
      {trigger}
      <Dropdown.Popover className="w-60">
        <Dropdown.Menu>
          <Dropdown.Item label="Settings" icon={Settings04} />
          <Dropdown.Item label="Log out" icon={UserLeft01} onAction={onSignOut} />
        </Dropdown.Menu>
      </Dropdown.Popover>
    </Dropdown.Root>
  );
}
