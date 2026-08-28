import { useTranslation } from "react-i18next";
import type { AvailablePlugin } from "@ora/contracts";
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@ora/ui";
import { localizeContractError } from "../../i18n/contract-error";
import { usePluginReadme } from "../../state/hooks/use-plugin-readme";
import { MarkdownDocument } from "../chat/markdown-message";
import { PluginLogo } from "./plugin-logo";

/** The marketplace detail page: breadcrumb back navigation plus the listing's rendered README. */
export function PluginReadmeView({
  plugin,
  onBack,
}: {
  plugin: AvailablePlugin;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  const readme = usePluginReadme(plugin.id);

  return (
    <div className="space-y-5">
      <Breadcrumb>
        <BreadcrumbList>
          <BreadcrumbItem>
            <BreadcrumbLink render={<button type="button" onClick={onBack} />}>
              {t("settings.plugins.title")}
            </BreadcrumbLink>
          </BreadcrumbItem>
          <BreadcrumbSeparator />
          <BreadcrumbItem>
            <BreadcrumbPage>{plugin.title || plugin.name}</BreadcrumbPage>
          </BreadcrumbItem>
        </BreadcrumbList>
      </Breadcrumb>

      <header className="flex items-start gap-3">
        <PluginLogo logo={plugin.logo} />
        <span className="min-w-0 flex-1">
          <span className="block text-lg font-semibold">
            {plugin.title || plugin.name}
          </span>
          <span className="mt-0.5 block truncate text-xs text-muted-foreground">
            {plugin.id} · {plugin.version} · {plugin.kind}
          </span>
        </span>
      </header>

      {readme.isLoading ? (
        <p className="py-10 text-center text-sm text-muted-foreground">
          {t("settings.plugins.readmeLoading")}
        </p>
      ) : readme.error !== null ? (
        <p className="py-10 text-center text-sm text-destructive">
          {t("settings.plugins.readmeFailed")}
          <span className="mt-1 block text-muted-foreground">
            {localizeContractError(readme.error, t)}
          </span>
        </p>
      ) : readme.data === undefined || readme.data.readme === null ? (
        <p className="py-10 text-center text-sm text-muted-foreground">
          {t("settings.plugins.readmeEmpty")}
        </p>
      ) : (
        <article>
          <MarkdownDocument content={readme.data.readme} />
        </article>
      )}
    </div>
  );
}
