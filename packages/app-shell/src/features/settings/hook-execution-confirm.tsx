import { useTranslation } from "react-i18next";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@ora/ui";

/** The package action whose authorization runs a Hook's `init` command. */
export type HookExecutionAction = "install" | "update";

/**
 * Discloses a Hook package's execution before the user authorizes it.
 *
 * A Hook ships a program the host runs with the current user's own permissions, outside Ora's
 * plugin sandbox, and this confirmation is where that becomes explicit rather than an
 * implementation detail (decision D5). The confirmation is also the authorization itself: the
 * action is only performed with the request flag this dialog produces, and the package lands
 * uninitialized if the user never answers it.
 */
export function HookExecutionConfirm({
  name,
  action,
  open,
  onOpenChange,
  onConfirm,
  busy,
}: {
  name: string;
  action: HookExecutionAction;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => void;
  busy: boolean;
}) {
  const { t } = useTranslation();
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("settings.plugins.hook.confirmTitle", { name })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t("settings.plugins.hook.executionDisclosure")}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction
            disabled={busy}
            onClick={(event) => {
              event.preventDefault();
              onConfirm();
            }}
          >
            {t(
              action === "install"
                ? "settings.plugins.hook.confirmInstall"
                : "settings.plugins.hook.confirmUpdate",
            )}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/**
 * States what removing a Hook does with its program and the configuration that program wrote.
 *
 * Removal is the one lifecycle event whose disclosure cannot be collapsed into the execution
 * notice: the command runs before the package disappears, and a tool that declares no `deinit`
 * leaves whatever it wrote in the Agent configuration behind, because Ora does not own those files.
 */
export function HookRemovalDisclosure() {
  const { t } = useTranslation();
  return (
    <p className="text-sm text-muted-foreground">
      {t("settings.plugins.hook.removalDisclosure")}
    </p>
  );
}
