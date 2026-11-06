import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";
import { useUiStore } from "../../state/stores/ui-store";
import type {
  WorkflowDependency,
  WorkflowDependencyFix,
  WorkflowDependencyStatus,
} from "./workflow-transfer";

const FIX_LABELS: Record<WorkflowDependencyFix, string> = {
  install: "settings.workflow.transfer.goInstall",
  configureMcp: "settings.workflow.transfer.goConfigure",
  manageMcp: "settings.workflow.transfer.goReview",
  reviewSkill: "settings.workflow.transfer.goReview",
};

/** Semantic pill tones shared by readiness (import) and binding state (export). */
type PillTone = "ok" | "warn" | "muted";

const PILL_TONES: Record<PillTone, string> = {
  ok: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
  warn: "bg-amber-500/10 text-amber-700 dark:text-amber-400",
  muted: "bg-muted text-muted-foreground",
};

const STATUS_TONES: Record<WorkflowDependencyStatus, PillTone> = {
  installed: "ok",
  missing: "warn",
  unavailable: "muted",
};

/** Status chip with a leading dot so state reads by shape as well as color. */
export function WorkflowTransferPill({
  tone,
  children,
}: {
  tone: PillTone;
  children: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium whitespace-nowrap",
        PILL_TONES[tone],
      )}
    >
      <span aria-hidden className="size-1.5 rounded-full bg-current" />
      {children}
    </span>
  );
}

interface WorkflowTransferDependenciesProps {
  title: string;
  dependencies: readonly WorkflowDependency[];
  /**
   * `import` shows local readiness, node usage, and a shortcut to fix it; `export` shows
   * the enabled state each reference is written with.
   */
  variant: "import" | "export";
  /**
   * Resolves a dependency instead of the shared Settings deep link. A host that already owns the
   * destination (the plugin import report navigates inside the pane it opened from) passes its
   * own handler; the editor import preview keeps the default.
   */
  onResolve?: (
    dependency: WorkflowDependency,
    fix: WorkflowDependencyFix,
  ) => void;
}

/** Lists the plugin references a workflow file carries. */
export function WorkflowTransferDependencies({
  title,
  dependencies,
  variant,
  onResolve,
}: WorkflowTransferDependenciesProps) {
  const { t } = useTranslation();
  const openSettingsAt = useUiStore((state) => state.openSettingsAt);
  const openPluginSettings = useUiStore((state) => state.openPluginSettings);

  /** Sends the user to the place that resolves this dependency. */
  function resolve(dependency: WorkflowDependency, fix: WorkflowDependencyFix) {
    if (onResolve !== undefined) {
      onResolve(dependency, fix);
      return;
    }
    switch (fix) {
      case "install":
        // Missing MCPs and Skills are installed from the marketplace first.
        openPluginSettings({ kind: "marketplaceSearch", query: dependency.id });
        return;
      case "configureMcp":
        openPluginSettings({
          kind: "configure",
          pluginId: dependency.id,
          displayName: dependency.label,
        });
        return;
      case "manageMcp":
        openPluginSettings({ kind: "manage" });
        return;
      case "reviewSkill":
        openSettingsAt("skills");
        return;
    }
  }
  const usable = dependencies.filter(
    (dependency) => dependency.status === "installed",
  ).length;

  return (
    <section
      aria-label={title}
      className="overflow-hidden rounded-[10px] border border-border"
    >
      <header className="flex items-center gap-2 border-b border-border px-3 py-2 text-[13px] font-medium">
        <span>{title}</span>
        <span className="ml-auto text-[11.5px] font-normal text-muted-foreground">
          {variant === "import"
            ? t("settings.workflow.transfer.dependencyCount", {
                usable,
                total: dependencies.length,
              })
            : t("settings.workflow.transfer.referenceCount", {
                total: dependencies.length,
              })}
        </span>
      </header>
      {dependencies.length === 0 ? (
        <p className="px-3 py-3 text-[12px] text-muted-foreground">
          {t("settings.workflow.transfer.noDependencies")}
        </p>
      ) : (
        <ul className="max-h-56 divide-y divide-border overflow-y-auto">
          {dependencies.map((dependency) => (
            <li
              key={`${dependency.kind}:${dependency.id}`}
              className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2.5 px-3 py-2"
            >
              <span className="w-10 rounded-[5px] bg-muted py-0.5 text-center text-[10px] font-semibold tracking-[0.05em] text-muted-foreground uppercase">
                {dependency.kind}
              </span>
              <div className="min-w-0">
                <p className="text-[13px] font-medium">{dependency.label}</p>
                <p className="font-mono text-[11px] break-all text-muted-foreground">
                  {dependency.id}
                </p>
                {variant === "import" && (
                  <p className="text-[11px] text-muted-foreground">
                    {t("settings.workflow.transfer.usedBy", {
                      nodes: dependency.nodeTitles.join(
                        t("settings.workflow.transfer.listSeparator"),
                      ),
                    })}
                  </p>
                )}
              </div>
              {variant === "import" ? (
                <div className="grid justify-items-end gap-1">
                  <WorkflowTransferPill tone={STATUS_TONES[dependency.status]}>
                    {t(
                      `settings.workflow.transfer.status.${dependency.status}`,
                    )}
                  </WorkflowTransferPill>
                  {dependency.fix !== null && (
                    <button
                      type="button"
                      className="text-[11.5px] text-blue-600 hover:underline dark:text-blue-400"
                      onClick={() => {
                        if (dependency.fix !== null) {
                          resolve(dependency, dependency.fix);
                        }
                      }}
                    >
                      {t(FIX_LABELS[dependency.fix])}
                    </button>
                  )}
                </div>
              ) : (
                <WorkflowTransferPill
                  tone={dependency.enabled ? "ok" : "muted"}
                >
                  {dependency.enabled
                    ? t("settings.workflow.transfer.bindingEnabled")
                    : t("settings.workflow.transfer.bindingDisabled")}
                </WorkflowTransferPill>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
