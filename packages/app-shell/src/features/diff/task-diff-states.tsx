import type { ReactNode } from "react";
import { IconCode } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";

interface DiffMessageProps {
  title: string;
  detail: string;
  action?: ReactNode;
}

/** Keeps the Changes layout stable while its first snapshot is being loaded. */
export function DiffLoadingState() {
  const { t } = useTranslation();
  return (
    <div
      className="flex min-h-0 flex-1 flex-col overflow-hidden bg-background"
      aria-busy="true"
    >
      <span role="status" className="sr-only">
        {t("diff.loading")}
      </span>
      <div className="flex min-h-0 flex-1 animate-pulse">
        <div className="min-w-0 flex-1 space-y-5 overflow-hidden px-4 py-3">
          {[0, 1, 2].map((index) => (
            <div key={index} className="space-y-2">
              <div className="h-7 rounded-md bg-muted/65" />
              <div className="space-y-1">
                <div className="h-5 rounded-sm bg-muted/35" />
                <div className="h-5 w-11/12 rounded-sm bg-muted/35" />
                <div className="h-5 w-4/5 rounded-sm bg-muted/35" />
              </div>
            </div>
          ))}
        </div>
        <aside className="w-60 shrink-0 space-y-3 border-l border-border px-3 py-3">
          <div className="h-3 w-16 rounded-full bg-muted" />
          <div className="h-6 w-4/5 rounded-sm bg-muted/55" />
          <div className="h-6 w-3/5 rounded-sm bg-muted/55" />
          <div className="h-6 w-11/12 rounded-sm bg-muted/55" />
        </aside>
      </div>
    </div>
  );
}

/** Shows a centered task-diff loading, empty, or error state. */
export function DiffMessage({ title, detail, action }: DiffMessageProps) {
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center p-8">
      <div className="max-w-sm text-center">
        <IconCode className="mx-auto size-6 text-muted-foreground" />
        <h2 className="mt-3 text-sm font-semibold">{title}</h2>
        <p className="mt-1 text-xs leading-5 text-muted-foreground">{detail}</p>
        {action && <div className="mt-4">{action}</div>}
      </div>
    </div>
  );
}
