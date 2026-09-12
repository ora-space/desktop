/** Renders a usage section title with its optional report age. */
export function UsageHeading({
  id,
  title,
  updatedLabel,
}: {
  id: string;
  title: string;
  updatedLabel?: string;
}) {
  return (
    <div className="flex items-baseline gap-2 pr-7">
      <h3 id={id} className="text-sm font-medium">
        {title}
      </h3>
      {updatedLabel && (
        <span className="text-[11px] text-muted-foreground">
          {updatedLabel}
        </span>
      )}
    </div>
  );
}

/** Renders one compact labeled value in the usage details grid. */
export function UsageMetric({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-md bg-muted/50 px-2 py-1.5">
      <div className="text-[10px] text-muted-foreground">{label}</div>
      <div className="font-medium tabular-nums">{value}</div>
    </div>
  );
}
