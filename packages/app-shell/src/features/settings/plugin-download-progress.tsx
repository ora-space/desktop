import type { PluginInstallProgress } from "../../platform";
import type { ReactNode } from "react";

/** Converts byte-level package transfer progress into a bounded percentage. */
function pluginDownloadPercentage(
  progress: PluginInstallProgress | null,
): number | null {
  if (progress?.total === null || progress === null || progress.total <= 0) {
    return null;
  }
  return Math.min(
    100,
    Math.round((progress.downloaded / progress.total) * 100),
  );
}

/** Renders determinate or indeterminate package progress inside a plugin action button. */
export function PluginDownloadProgress({
  progress,
  label,
  children,
}: {
  progress: PluginInstallProgress | null;
  label: string;
  children?: ReactNode;
}) {
  const value = pluginDownloadPercentage(progress);
  const radius = 11;
  const circumference = 2 * Math.PI * radius;
  const offset =
    value === null ? 0 : circumference * (1 - Math.max(0, value) / 100);

  return (
    <span
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value ?? undefined}
      className="relative grid size-7 place-items-center"
    >
      <svg
        viewBox="0 0 28 28"
        aria-hidden="true"
        className={
          value === null
            ? "size-7 -rotate-90 animate-spin"
            : "size-7 -rotate-90"
        }
      >
        <circle
          cx="14"
          cy="14"
          r={radius}
          fill="none"
          strokeWidth="2.5"
          className="stroke-muted"
        />
        <circle
          cx="14"
          cy="14"
          r={radius}
          fill="none"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeDasharray={
            value === null ? `18 ${circumference}` : circumference
          }
          strokeDashoffset={offset}
          className="stroke-primary transition-[stroke-dashoffset] duration-300 ease-out"
        />
      </svg>
      {children !== undefined && (
        <span className="absolute inset-0 grid place-items-center">
          {children}
        </span>
      )}
    </span>
  );
}
