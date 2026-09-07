"use client";

import * as React from "react";

// --- UI Primitives ---
import { Button } from "../primitive/button/index.tsx";

// --- Icons ---
import { MoonStarIcon } from "../icons/moon-star-icon.tsx";
import { SunIcon } from "../icons/sun-icon.tsx";

/** Reads the current document / OS color scheme. Used as the ThemeToggle initial state. */
function readInitialDarkMode(): boolean {
  if (typeof document === "undefined") return false;
  return (
    !!document.querySelector('meta[name="color-scheme"][content="dark"]') ||
    window.matchMedia("(prefers-color-scheme: dark)").matches
  );
}

export function ThemeToggle() {
  const [isDarkMode, setIsDarkMode] = React.useState(readInitialDarkMode);

  React.useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const handleChange = () => setIsDarkMode(mediaQuery.matches);
    mediaQuery.addEventListener("change", handleChange);
    return () => mediaQuery.removeEventListener("change", handleChange);
  }, []);

  React.useEffect(() => {
    document.documentElement.classList.toggle("dark", isDarkMode);
  }, [isDarkMode]);

  const toggleDarkMode = () => setIsDarkMode((isDark) => !isDark);

  return (
    <Button
      onClick={toggleDarkMode}
      aria-label={`Switch to ${isDarkMode ? "light" : "dark"} mode`}
      data-style="ghost"
    >
      {isDarkMode ? (
        <MoonStarIcon className="tiptap-button-icon" />
      ) : (
        <SunIcon className="tiptap-button-icon" />
      )}
    </Button>
  );
}
