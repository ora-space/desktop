import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
import { usePlatform } from "../../platform";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  ScrollArea,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  cn,
} from "@ora/ui";
import {
  IconAdjustments,
  IconBug,
  IconCheck,
  IconDatabase,
  IconDeviceDesktop,
  IconFolder,
  IconLanguage,
  IconMoon,
  IconProng,
  IconPuzzle,
  IconRobot,
  IconSparkles,
  IconSun,
} from "@tabler/icons-react";
import type { Locale } from "../../i18n/i18n";
import { RolesSettings, SkillsSettings } from "./atoms-settings";
import { PluginsSettings } from "./plugins-settings";
import type { PluginConfigurationNavigationGuard } from "./plugin-configuration-editor";
import { SettingsHeading } from "./settings-heading";
import { RuntimeLogLevelSettings } from "./runtime-log-level-settings";
import { ProxySettings } from "./proxy-settings";
import { DeveloperModeSettings } from "./developer-mode-settings";
import { useDeveloperMode } from "../../state/hooks/use-developer-mode";
import { useUiStore, type SettingsCategory } from "../../state/stores/ui-store";
import {
  useSettingsStore,
  type SettingsPreferences,
} from "../../state/stores/settings-store";
import { DesktopUpdateControl } from "../workspace/desktop-update-control";
import type { ThemeMode } from "../../state/stores/settings-store";

type PendingSettingsNavigation =
  { kind: "close" } | { kind: "category"; category: SettingsCategory };

/** Presents shared Ora preferences in a dense IDE-style settings surface. */
export function SettingsDialog() {
  const { t } = useTranslation();
  const open = useUiStore((s) => s.settingsOpen);
  const setOpen = useUiStore((s) => s.setSettingsOpen);
  const category = useUiStore((s) => s.settingsCategory);
  const setCategory = useUiStore((s) => s.setSettingsCategory);
  const settings = useSettingsStore((s) => s.settings);
  const updateSettings = useSettingsStore((s) => s.updateSettings);
  const [pendingNavigation, setPendingNavigation] =
    useState<PendingSettingsNavigation | null>(null);
  const pluginConfigurationGuard =
    useRef<PluginConfigurationNavigationGuard | null>(null);
  const developerMode = useDeveloperMode();
  const developerModeEnabled = developerMode.state?.enabled === true;

  const registerPluginConfigurationGuard = useCallback(
    (guard: PluginConfigurationNavigationGuard | null) => {
      pluginConfigurationGuard.current = guard;
    },
    [],
  );

  /** Applies a navigation request after its unsaved-change decision has completed. */
  const applyNavigation = (navigation: PendingSettingsNavigation) => {
    setPendingNavigation(null);
    if (navigation.kind === "close") setOpen(false);
    else setCategory(navigation.category);
  };

  /** Defers Settings navigation while the active plugin editor owns unsaved input. */
  const requestNavigation = (navigation: PendingSettingsNavigation) => {
    if (pluginConfigurationGuard.current?.isDirty() === true) {
      setPendingNavigation(navigation);
      return;
    }
    applyNavigation(navigation);
  };

  const categories: Array<{
    id: SettingsCategory;
    icon: typeof IconAdjustments;
    label: string;
  }> = [
    {
      id: "appearance",
      icon: IconAdjustments,
      label: t("settings.nav.appearance"),
    },
    { id: "roles", icon: IconRobot, label: t("settings.nav.roles") },
    { id: "skills", icon: IconSparkles, label: t("settings.nav.skills") },
    { id: "plugins", icon: IconPuzzle, label: t("settings.nav.plugins") },
    { id: "proxy", icon: IconProng, label: t("settings.nav.proxy") },
    { id: "privacy", icon: IconDatabase, label: t("settings.nav.privacy") },
    {
      id: "developer",
      icon: IconBug,
      label: t("settings.nav.developer"),
    },
  ];

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={(nextOpen) => {
          if (nextOpen) setOpen(true);
          else requestNavigation({ kind: "close" });
        }}
      >
        <DialogContent
          showCloseButton
          className="h-[min(720px,calc(100dvh-2rem))] w-[min(1040px,calc(100vw-2rem))] max-w-none gap-0 overflow-hidden p-0 transition-[width,height] duration-200 sm:max-w-none"
        >
          <DialogHeader className="sr-only">
            <DialogTitle>{t("common.settings")}</DialogTitle>
            <DialogDescription>{t("settings.description")}</DialogDescription>
          </DialogHeader>
          <div className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] sm:grid-rows-1 sm:grid-cols-[210px_minmax(0,1fr)]">
            <aside className="flex flex-col border-b border-border bg-muted/35 p-3 sm:border-b-0 sm:border-r">
              <div className="hidden h-11 items-center gap-2 px-2 sm:flex">
                <div className="flex size-7 items-center justify-center rounded-md bg-foreground text-background">
                  <IconAdjustments className="size-4" />
                </div>
                <span className="text-sm font-semibold">
                  {t("common.settings")}
                </span>
              </div>
              <nav
                className="flex gap-1 overflow-x-auto p-1 sm:mt-2 sm:flex-col"
                aria-label={t("common.settings")}
              >
                {categories.map((item) => {
                  const Icon = item.icon;
                  return (
                    <button
                      key={item.id}
                      type="button"
                      onClick={() => {
                        if (item.id !== category)
                          requestNavigation({
                            kind: "category",
                            category: item.id,
                          });
                      }}
                      className={cn(
                        "flex h-9 shrink-0 items-center gap-2 rounded-md px-2.5 text-left text-sm font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring sm:w-full",
                        category === item.id
                          ? "bg-background text-foreground shadow-sm ring-1 ring-border"
                          : "text-muted-foreground hover:bg-background/70 hover:text-foreground",
                      )}
                    >
                      <Icon className="size-4" />
                      <span className="truncate">{item.label}</span>
                    </button>
                  );
                })}
              </nav>
              <div className="mt-auto hidden pt-6 sm:block">
                <DesktopUpdateControl />
              </div>
            </aside>

            <ScrollArea className="min-h-0">
              <div className="mx-auto w-full max-w-3xl p-5 pb-12 sm:p-8 sm:pb-12">
                {category === "appearance" && (
                  <AppearanceSettings
                    settings={settings}
                    onUpdate={updateSettings}
                  />
                )}
                {category === "roles" && <RolesSettings />}
                {category === "skills" && <SkillsSettings />}
                {category === "plugins" && (
                  <PluginsSettings
                    onNavigationGuardChange={registerPluginConfigurationGuard}
                  />
                )}
                {category === "proxy" && <ProxySettings />}
                {category === "privacy" && <PrivacySettings />}
                {category === "developer" && (
                  <DeveloperSettings
                    developerMode={developerMode}
                    developerModeEnabled={developerModeEnabled}
                  />
                )}
              </div>
            </ScrollArea>
          </div>
        </DialogContent>
      </Dialog>
      <AlertDialog
        open={pendingNavigation !== null}
        onOpenChange={(nextOpen) => {
          if (!nextOpen) setPendingNavigation(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.plugins.configuration.unsavedTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.plugins.configuration.unsavedDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <Button
              variant="outline"
              onClick={() => {
                if (pendingNavigation !== null)
                  applyNavigation(pendingNavigation);
              }}
            >
              {t("settings.plugins.configuration.discard")}
            </Button>
            <Button
              onClick={() => {
                const navigation = pendingNavigation;
                if (navigation === null) return;
                void pluginConfigurationGuard.current
                  ?.save()
                  .then((saved) => saved && applyNavigation(navigation));
              }}
            >
              {t("settings.plugins.configuration.save")}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

/** Applies visual and locale preferences immediately so users can evaluate the result in context. */
function AppearanceSettings({
  settings,
  onUpdate,
}: {
  settings: SettingsPreferences;
  onUpdate: (patch: Partial<SettingsPreferences>) => void;
}) {
  const { i18n, t } = useTranslation();
  const locale: Locale = i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";
  const themes: Array<{
    value: ThemeMode;
    icon: typeof IconSun;
    label: string;
  }> = [
    {
      value: "system",
      icon: IconDeviceDesktop,
      label: t("settings.appearance.system"),
    },
    { value: "light", icon: IconSun, label: t("settings.appearance.light") },
    { value: "dark", icon: IconMoon, label: t("settings.appearance.dark") },
  ];

  return (
    <div className="space-y-7">
      <SettingsHeading title={t("settings.appearance.title")} />
      <SettingsGroup
        title={t("settings.appearance.theme")}
        description={t("settings.appearance.themeDescription")}
      >
        <div className="grid grid-cols-3 gap-2">
          {themes.map((theme) => {
            const Icon = theme.icon;
            const selected = settings.theme === theme.value;
            return (
              <button
                key={theme.value}
                type="button"
                aria-pressed={selected}
                onClick={() => onUpdate({ theme: theme.value })}
                className={cn(
                  "relative overflow-hidden rounded-md border p-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  selected
                    ? "border-foreground"
                    : "border-border hover:border-foreground/40",
                )}
              >
                <div
                  className={cn(
                    "mb-2 h-14 rounded-sm border p-1.5",
                    theme.value === "dark"
                      ? "border-zinc-700 bg-zinc-950"
                      : theme.value === "light"
                        ? "bg-white"
                        : "bg-gradient-to-r from-white from-50% to-zinc-950 to-50%",
                  )}
                >
                  <div
                    className={cn(
                      "h-1.5 w-8 rounded-full",
                      theme.value === "dark" ? "bg-zinc-600" : "bg-zinc-300",
                    )}
                  />
                  <div
                    className={cn(
                      "mt-2 h-5 rounded-sm border",
                      theme.value === "dark"
                        ? "border-zinc-700 bg-zinc-900"
                        : "border-zinc-200 bg-zinc-50",
                    )}
                  />
                </div>
                <span className="flex items-center gap-1.5 text-xs font-medium">
                  <Icon className="size-3.5" />
                  {theme.label}
                </span>
                {selected && (
                  <IconCheck className="absolute right-2 top-2 size-3.5" />
                )}
              </button>
            );
          })}
        </div>
      </SettingsGroup>
      <SettingsRow
        icon={IconLanguage}
        title={t("settings.appearance.language")}
        description={t("settings.appearance.languageDescription")}
      >
        <Select
          value={locale}
          onValueChange={(value) => void i18n.changeLanguage(value as Locale)}
        >
          <SelectTrigger className="w-40">
            <span className="flex-1 text-left">
              {locale === "zh-CN"
                ? t("account.switchChinese")
                : t("account.switchEnglish")}
            </span>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="zh-CN">{t("account.switchChinese")}</SelectItem>
            <SelectItem value="en-US">{t("account.switchEnglish")}</SelectItem>
          </SelectContent>
        </Select>
      </SettingsRow>
    </div>
  );
}

/** Configures where newly created worktrees are stored. */
function PrivacySettings() {
  const { t } = useTranslation();
  const platform = usePlatform();
  const [worktreeRoot, setWorktreeRoot] = useState<string | null>(null);
  const [worktreeError, setWorktreeError] = useState<string | null>(null);
  const [worktreeSaving, setWorktreeSaving] = useState(false);
  const worktreeStorage = platform.worktreeStorage;

  useEffect(() => {
    let active = true;
    worktreeStorage.getRoot().then(
      (root) => {
        if (active) {
          setWorktreeRoot(root);
          setWorktreeError(null);
        }
      },
      (error: unknown) => {
        if (active) {
          setWorktreeError(localizeContractError(error, t));
        }
      },
    );
    return () => {
      active = false;
    };
  }, [t, worktreeStorage]);

  async function changeWorktreeRoot(): Promise<void> {
    setWorktreeError(null);
    try {
      const selected = await platform.selectPath({
        kind: "directory",
        initialPath: worktreeRoot ?? undefined,
      });
      if (selected === null) {
        return;
      }

      setWorktreeSaving(true);
      await worktreeStorage.setRoot(selected);
      setWorktreeRoot(selected);
    } catch (error: unknown) {
      setWorktreeError(localizeContractError(error, t));
    } finally {
      setWorktreeSaving(false);
    }
  }

  return (
    <div className="space-y-7">
      <SettingsHeading title={t("settings.privacy.title")} />
      <SettingsRow
        icon={IconFolder}
        title={t("settings.privacy.worktreeRoot")}
        description={t("settings.privacy.worktreeRootDescription")}
      >
        <div className="flex max-w-sm flex-col items-end gap-2">
          <code
            data-selectable
            className="max-w-full break-all text-right text-xs text-muted-foreground"
          >
            {worktreeRoot ?? t("settings.privacy.worktreeRootLoading")}
          </code>
          <Button
            variant="outline"
            disabled={worktreeRoot === null || worktreeSaving}
            onClick={changeWorktreeRoot}
          >
            <IconFolder />
            {worktreeSaving
              ? t("common.saving")
              : t("settings.privacy.changeWorktreeRoot")}
          </Button>
          {worktreeError !== null && (
            <p data-selectable className="text-right text-xs text-destructive">
              {worktreeError}
            </p>
          )}
        </div>
      </SettingsRow>
    </div>
  );
}

/** Groups settings intended for diagnosis and development without treating them as authorization. */
function DeveloperSettings({
  developerMode,
  developerModeEnabled,
}: {
  developerMode: ReturnType<typeof useDeveloperMode>;
  developerModeEnabled: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-7">
      <SettingsHeading
        title={t("settings.developer.title")}
        description={t("settings.developer.description")}
      />
      <DeveloperModeSettings controller={developerMode} />
      {developerModeEnabled && <RuntimeLogLevelSettings />}
    </div>
  );
}

/** Labels a grouped control without introducing nested decorative cards. */
function SettingsGroup({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <section>
      <div className="mb-3">
        <h3 className="text-sm font-medium">{title}</h3>
        <p className="mt-1 text-xs leading-5 text-muted-foreground">
          {description}
        </p>
      </div>
      {children}
    </section>
  );
}

/** Aligns a preference description with its compact trailing control. */
function SettingsRow({
  icon: Icon,
  title,
  description,
  children,
}: {
  icon: typeof IconAdjustments;
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-3 border-y border-border py-4 sm:flex-row sm:items-center">
      <Icon className="hidden size-4 shrink-0 text-muted-foreground sm:block" />
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{title}</p>
        <p className="mt-1 text-xs leading-5 text-muted-foreground">
          {description}
        </p>
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}
