import type { ReactNode } from "react";

export function Section({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <section className="mb-12">
      <h2 className="text-xs font-semibold uppercase tracking-widest text-fg-secondary mb-4 pb-2 border-b border-border">
        {title}
      </h2>
      {children}
    </section>
  );
}

export function Row({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center gap-6 py-3 border-b border-border-subtle last:border-0">
      <span className="w-28 shrink-0 text-xs text-fg-secondary">{label}</span>
      <div className="flex items-center gap-3 flex-wrap">{children}</div>
    </div>
  );
}
