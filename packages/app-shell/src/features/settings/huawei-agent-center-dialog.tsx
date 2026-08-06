import { useTranslation } from "react-i18next";
import {
  Button,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@ora/ui";
import { IconExternalLink } from "@tabler/icons-react";

interface HuaweiAgentCenterDialogProps {
  open: boolean;
  available: boolean;
  opening: boolean;
  compatibilityTestOpening: boolean;
  connectionFailed: boolean;
  compatibilityTestFailed: boolean;
  onOpenChange(open: boolean): void;
  onLaunch(): void;
  onLaunchCompatibilityTest(): void;
}

/** Presents plain-text integration guidance before launching the Huawei internal WebView. */
export function HuaweiAgentCenterDialog({
  open,
  available,
  opening,
  compatibilityTestOpening,
  connectionFailed,
  compatibilityTestFailed,
  onOpenChange,
  onLaunch,
  onLaunchCompatibilityTest,
}: HuaweiAgentCenterDialogProps) {
  const { t } = useTranslation();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100vh-2rem)] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{t("settings.skills.huaweiDialogTitle")}</DialogTitle>
          <DialogDescription>{t("settings.skills.huaweiDialogDescription")}</DialogDescription>
        </DialogHeader>

        <p className="whitespace-pre-line text-sm leading-6 text-muted-foreground">
          {t("settings.skills.huaweiPlainTextGuide")}
        </p>

        {connectionFailed && (
          <p className="text-sm text-destructive" role="alert">
            {t("settings.skills.huaweiConnectionFailed")}
          </p>
        )}
        {compatibilityTestFailed && (
          <p className="text-sm text-destructive" role="alert">
            {t("settings.skills.huaweiCompatibilityTestFailed")}
          </p>
        )}
        {!available && (
          <p className="text-sm text-muted-foreground" role="status">
            {t("settings.skills.huaweiDesktopRequired")}
          </p>
        )}

        <DialogFooter>
          <DialogClose render={<Button type="button" variant="ghost" />}>
            {t("settings.skills.huaweiClose")}
          </DialogClose>
          <Button
            type="button"
            variant="secondary"
            disabled={!available || opening || compatibilityTestOpening}
            onClick={onLaunchCompatibilityTest}
          >
            <IconExternalLink aria-hidden="true" />
            {compatibilityTestOpening
              ? t("settings.skills.huaweiCompatibilityTestOpening")
              : t("settings.skills.huaweiCompatibilityTestLaunch")}
          </Button>
          <Button
            type="button"
            disabled={!available || opening || compatibilityTestOpening}
            onClick={onLaunch}
          >
            <IconExternalLink aria-hidden="true" />
            {opening
              ? t("settings.skills.marketplaceOpening")
              : t("settings.skills.huaweiLaunch")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
