import { useTranslation } from "react-i18next";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@ora/ui";
import { IconCheck, IconChevronDown, IconShieldCheck, IconShieldHalf, IconShieldLock } from "@tabler/icons-react";
import type { ComponentType } from "react";
import type { ApprovalPolicy } from "../../state/stores/settings-store";
import { useSettingsStore } from "../../state/stores/settings-store";

/** The approval policies offered in the composer, ordered from most cautious to most permissive. */
const POLICIES: readonly ApprovalPolicy[] = ["always", "risky", "trusted"] as const;

const POLICY_ICONS: Record<ApprovalPolicy, ComponentType<{ className?: string }>> = {
  always: IconShieldLock,
  risky: IconShieldHalf,
  trusted: IconShieldCheck,
};

const POLICY_LABEL_KEYS: Record<ApprovalPolicy, string> = {
  always: "chat.permission.always",
  risky: "chat.permission.risky",
  trusted: "chat.permission.trusted",
};

/**
 * The composer's permission-mode picker, sitting at the footer's left edge. It mirrors the
 * `approvalPolicy` setting so switching here and in Settings stays in sync. The most permissive
 * policy ("trusted" / full access) stays visually neutral because it is a normal operating mode;
 * its explicit shield icon and label communicate the scope without presenting a false warning.
 */
export function PermissionSelector({ disabled = false }: { disabled?: boolean }) {
  const { t } = useTranslation();
  const approvalPolicy = useSettingsStore((state) => state.settings.approvalPolicy);
  const updateSettings = useSettingsStore((state) => state.updateSettings);

  const ActiveIcon = POLICY_ICONS[approvalPolicy];
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
            className="h-7 gap-1.5 rounded-md px-2 text-xs font-normal text-muted-foreground hover:bg-muted/60 hover:text-foreground"
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
              className="gap-1.5 rounded-sm px-2 py-1.5 text-xs"
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
