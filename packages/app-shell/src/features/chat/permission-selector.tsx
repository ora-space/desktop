import { useTranslation } from "react-i18next";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@ora/ui";
import { IconAlertTriangle, IconCheck, IconChevronDown, IconShieldHalf, IconShieldLock } from "@tabler/icons-react";
import type { ComponentType } from "react";
import type { ApprovalPolicy } from "../../state/stores/settings-store";
import { useSettingsStore } from "../../state/stores/settings-store";

/** The approval policies offered in the composer, ordered from most cautious to most permissive. */
const POLICIES: readonly ApprovalPolicy[] = ["always", "risky", "trusted"] as const;

const POLICY_ICONS: Record<ApprovalPolicy, ComponentType<{ className?: string }>> = {
  always: IconShieldLock,
  risky: IconShieldHalf,
  trusted: IconAlertTriangle,
};

const POLICY_LABEL_KEYS: Record<ApprovalPolicy, string> = {
  always: "chat.permission.always",
  risky: "chat.permission.risky",
  trusted: "chat.permission.trusted",
};

/**
 * The composer's permission-mode picker, sitting at the footer's left edge. It mirrors the
 * `approvalPolicy` setting so switching here and in Settings stays in sync. The most permissive
 * policy ("trusted" / full access) is tinted amber to flag that the agent may act without asking.
 */
export function PermissionSelector({ disabled = false }: { disabled?: boolean }) {
  const { t } = useTranslation();
  const approvalPolicy = useSettingsStore((state) => state.settings.approvalPolicy);
  const updateSettings = useSettingsStore((state) => state.updateSettings);

  const ActiveIcon = POLICY_ICONS[approvalPolicy];
  const isFullAccess = approvalPolicy === "trusted";

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={disabled}
            aria-label={t("chat.permission.label")}
            className={
              isFullAccess
                ? "h-7 gap-1.5 rounded-md px-2 text-xs font-normal text-amber-600 hover:text-amber-600 hover:bg-amber-500/10 dark:text-amber-500 dark:hover:text-amber-500"
                : "h-7 gap-1.5 rounded-md px-2 text-xs font-normal text-muted-foreground hover:text-foreground"
            }
          />
        }
      >
        <ActiveIcon className="size-3.5 shrink-0" />
        <span className="whitespace-nowrap">{t(POLICY_LABEL_KEYS[approvalPolicy])}</span>
        <IconChevronDown className="size-3 shrink-0 opacity-50" aria-hidden="true" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side="top" className="w-44">
        {POLICIES.map((policy) => {
          const Icon = POLICY_ICONS[policy];
          // Only full access ("trusted") is wired up for now; the stricter
          // policies are shown but disabled so the option set stays visible.
          const selectable = policy === "trusted";
          return (
            <DropdownMenuItem
              key={policy}
              disabled={!selectable}
              className={
                policy === "trusted"
                  ? "gap-1.5 rounded-sm px-2 py-1.5 text-xs text-amber-600 dark:text-amber-500"
                  : "gap-1.5 rounded-sm px-2 py-1.5 text-xs"
              }
              onClick={() => updateSettings({ approvalPolicy: policy })}
            >
              <Icon className="size-3.5 shrink-0" />
              {t(POLICY_LABEL_KEYS[policy])}
              {policy === approvalPolicy && <IconCheck className="ml-auto size-4" />}
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
