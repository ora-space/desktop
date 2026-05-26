import { Sun, Monitor, Moon } from "lucide-react";
import { cn } from "./utils";
import { useTheme, type Theme } from "./use-theme";

const OPTIONS: { value: Theme; icon: React.ReactNode }[] = [
  { value: "light", icon: <Sun className="h-3.5 w-3.5" /> },
  { value: "system", icon: <Monitor className="h-3.5 w-3.5" /> },
  { value: "dark", icon: <Moon className="h-3.5 w-3.5" /> },
];

export function ThemeToggle() {
  const [theme, setTheme] = useTheme();

  return (
    <div className="inline-flex items-center gap-0.5 rounded-full border border-border bg-bg-subtle p-0.5">
      {OPTIONS.map(({ value, icon }) => (
        <button
          key={value}
          type="button"
          aria-label={value}
          onClick={() => setTheme(value)}
          className={cn(
            "flex h-6 w-6 items-center justify-center rounded-full transition-colors",
            theme === value
              ? "bg-bg text-fg shadow-sm"
              : "text-fg-secondary hover:text-fg",
          )}
        >
          {icon}
        </button>
      ))}
    </div>
  );
}
