import { IconChartHistogram } from "@tabler/icons-react";
import {
  Button,
  toast,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@ora/ui";
import { useTranslation } from "react-i18next";
import { usePlatform } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";

/** Opens the Dashboard with a host-issued trace grant for the selected chat session. */
export function SessionDashboardButton({
  sessionId,
}: {
  sessionId: string | null;
}) {
  const { t } = useTranslation();
  const { surfaces } = usePlatform();
  if (sessionId === null) return null;

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="icon"
            aria-label={t("chat.dashboard.open")}
            title={t("chat.dashboard.open")}
            onClick={() => {
              const { embeddedSupported, setSidePanelInstance, applyEvent } =
                useSurfaceStore.getState();
              const target = embeddedSupported ? "embedded" : "windowed";
              void surfaces
                .openSessionTraceDashboard(sessionId, target)
                .then((record) => {
                  applyEvent({ type: "opened", ...record });
                  if (record.target === "embedded") {
                    setSidePanelInstance(record.instance);
                  }
                })
                .catch(() => toast.error(t("surface.openFailed")));
            }}
          >
            <IconChartHistogram />
          </Button>
        }
      />
      <TooltipContent>{t("chat.dashboard.open")}</TooltipContent>
    </Tooltip>
  );
}
