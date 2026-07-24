import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Switch,
} from "@ora/ui";
import {
  IconArrowRight,
  IconCheck,
  IconDots,
  IconDownload,
  IconExternalLink,
  IconPlayerPlay,
  IconTrash,
} from "@tabler/icons-react";
import type { PluginEntry } from "./plugin-catalog";
import { PluginTile } from "./plugin-tile";

/** The prompt suggestions and feature bullets are generic; each is filled with the plugin's name. */
const SAMPLE_PROMPT_KEYS = ["settings.plugins.prompt1", "settings.plugins.prompt2", "settings.plugins.prompt3"];
const FEATURE_KEYS = [
  "settings.plugins.feature1",
  "settings.plugins.feature2",
  "settings.plugins.feature3",
  "settings.plugins.feature4",
];
const RESOURCE_KEYS = [
  "settings.plugins.repository",
  "settings.plugins.issues",
  "settings.plugins.license",
  "settings.plugins.marketplace",
];

/**
 * A single plugin's page: identity and install state up top, then the sample prompts,
 * the skills it contributes, a details section and a VS Code-style info table. Every
 * value is read from the hard-coded catalog; the skill switches are local-only.
 */
export function PluginDetail({ plugin, installed, onBack, onToggleInstall }: {
  plugin: PluginEntry;
  installed: boolean;
  onBack: () => void;
  onToggleInstall: () => void;
}) {
  const { i18n, t } = useTranslation();
  const [disabledSkills, setDisabledSkills] = useState<string[]>([]);

  const toggleSkill = (skill: string) => setDisabledSkills((skills) => (
    skills.includes(skill) ? skills.filter((current) => current !== skill) : [...skills, skill]
  ));

  const summary = t(plugin.summaryKey);
  const updated = new Intl.DateTimeFormat(i18n.resolvedLanguage, { dateStyle: "medium" }).format(new Date(plugin.updated));

  return (
    <div className="space-y-6">
      <Breadcrumb>
        <BreadcrumbList>
          <BreadcrumbItem>
            <BreadcrumbLink render={<button type="button" onClick={onBack} />}>{t("settings.plugins.title")}</BreadcrumbLink>
          </BreadcrumbItem>
          <BreadcrumbSeparator />
          <BreadcrumbItem><BreadcrumbPage>{plugin.name}</BreadcrumbPage></BreadcrumbItem>
        </BreadcrumbList>
      </Breadcrumb>

      <header className="flex items-start gap-4">
        <PluginTile plugin={plugin} size="xl" />
        <div className="min-w-0 flex-1">
          <h2 className="text-xl font-semibold">{plugin.name}</h2>
          <p className="mt-1 text-sm text-muted-foreground">{summary}</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {installed
            ? (
              <>
                <DropdownMenu>
                  <DropdownMenuTrigger
                    render={<Button variant="ghost" size="icon-sm" aria-label={t("settings.plugins.openMenu", { name: plugin.name })} className="text-muted-foreground" />}
                  >
                    <IconDots />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" className="w-40">
                    <DropdownMenuItem variant="destructive" onClick={onToggleInstall}><IconTrash />{t("settings.plugins.uninstall")}</DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
                <Button><IconPlayerPlay />{t("settings.plugins.tryNow")}</Button>
              </>
            )
            : <Button variant="outline" onClick={onToggleInstall}><IconDownload />{t("settings.plugins.install")}</Button>}
        </div>
      </header>

      <div className="rounded-xl border border-border bg-gradient-to-br from-muted/70 via-muted/30 to-background p-6">
        <div className="mx-auto flex max-w-sm flex-col gap-2.5">
          {SAMPLE_PROMPT_KEYS.map((key) => (
            <div key={key} className="flex items-center gap-2 rounded-lg border border-border bg-background/80 px-3 py-2 text-xs shadow-sm">
              <PluginTile plugin={plugin} size="sm" className="size-4 [&_svg]:size-4" />
              <span className="shrink-0 font-medium">{plugin.name}</span>
              <span className="min-w-0 flex-1 truncate text-muted-foreground">{t(key, { name: plugin.name })}</span>
              <IconArrowRight className="size-3.5 shrink-0 text-muted-foreground" />
            </div>
          ))}
        </div>
      </div>

      <p data-selectable className="text-sm leading-6 text-muted-foreground">
        {t("settings.plugins.overview", { name: plugin.name, publisher: plugin.publisher, summary })}
      </p>

      <PluginSection title={t("settings.plugins.skills")} count={plugin.skills.length}>
        <div className="divide-y divide-border">
          {plugin.skills.map((skill) => (
            <div key={skill} className="flex items-center gap-3 py-3">
              <PluginTile plugin={plugin} size="sm" />
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium">{skill}</p>
                <p className="mt-0.5 truncate text-xs text-muted-foreground">{t("settings.plugins.skillSummary", { name: skill })}</p>
              </div>
              <Switch
                checked={!disabledSkills.includes(skill)}
                onCheckedChange={() => toggleSkill(skill)}
                aria-label={t("settings.plugins.toggleSkill", { name: skill })}
              />
            </div>
          ))}
        </div>
      </PluginSection>

      <PluginSection title={t("settings.plugins.details")}>
        <ul data-selectable className="space-y-2 pt-3">
          {FEATURE_KEYS.map((key) => (
            <li key={key} className="flex gap-2 text-sm leading-6 text-muted-foreground">
              <IconCheck className="mt-1.5 size-3.5 shrink-0 text-foreground/60" />
              {t(key, { name: plugin.name })}
            </li>
          ))}
        </ul>
      </PluginSection>

      <PluginSection title={t("settings.plugins.info")}>
        <dl className="divide-y divide-border text-sm">
          <InfoRow label={t("settings.plugins.identifier")}>
            <code data-selectable className="break-all font-mono text-xs">{plugin.identifier}</code>
          </InfoRow>
          <InfoRow label={t("settings.plugins.version")}>{plugin.version}</InfoRow>
          <InfoRow label={t("settings.plugins.lastUpdated")}>{updated}</InfoRow>
          <InfoRow label={t("settings.plugins.size")}>{plugin.size}</InfoRow>
          <InfoRow label={t("settings.plugins.publisher")}>{plugin.publisher}</InfoRow>
          <InfoRow label={t("settings.plugins.category")}>{t(plugin.categoryKey)}</InfoRow>
          <InfoRow label={t("settings.plugins.capabilities")}>{plugin.capabilityKeys.map((key) => t(key)).join(t("settings.plugins.listSeparator"))}</InfoRow>
        </dl>
        <div className="flex flex-wrap gap-1.5 pt-3">
          {RESOURCE_KEYS.map((key) => (
            <Button key={key} variant="outline" size="sm" disabled>
              <IconExternalLink />
              {t(key)}
            </Button>
          ))}
        </div>
      </PluginSection>
    </div>
  );
}

/** Gives the detail page's sections one underlined heading style, optionally with a count. */
function PluginSection({ title, count, children }: { title: string; count?: number; children: React.ReactNode }) {
  return (
    <section>
      <h3 className="flex items-baseline gap-2 border-b border-border pb-2 text-sm font-medium">
        {title}
        {count !== undefined && <span className="font-normal text-muted-foreground">{count}</span>}
      </h3>
      {children}
    </section>
  );
}

/** One label/value pair of the info table, laid out as a fixed label column like VS Code's. */
function InfoRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-4 py-2.5">
      <dt className="w-32 shrink-0 text-muted-foreground">{label}</dt>
      <dd className="min-w-0 flex-1">{children}</dd>
    </div>
  );
}
