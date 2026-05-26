import { useState, useEffect } from "react";

export type Theme = "light" | "system" | "dark";

const STORAGE_KEY = "ora-theme";

export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(
    () => (localStorage.getItem(STORAGE_KEY) as Theme) ?? "system",
  );

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem(STORAGE_KEY, theme);
  }, [theme]);

  return [theme, setThemeState] as const;
}
