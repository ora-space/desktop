import { useState } from "react";
import { Button, ThemeToggle } from "@ora/ui";
import ButtonPage from "./pages/ButtonPage";
import InputPage from "./pages/InputPage";
import CardPage from "./pages/CardPage";
import CheckboxPage from "./pages/CheckboxPage";
import AlertPage from "./pages/AlertPage";
import AvatarPage from "./pages/AvatarPage";
import BadgePage from "./pages/BadgePage";

const NAV_ITEMS = [
  { id: "button", label: "Button", page: <ButtonPage /> },
  { id: "input", label: "Input", page: <InputPage /> },
  { id: "card", label: "Card", page: <CardPage /> },
  { id: "checkbox", label: "Checkbox", page: <CheckboxPage /> },
  { id: "alert", label: "Alert", page: <AlertPage /> },
  { id: "avatar", label: "Avatar", page: <AvatarPage /> },
  { id: "badge", label: "Badge", page: <BadgePage /> },
] as const;

type NavId = (typeof NAV_ITEMS)[number]["id"];

export default function App() {
  const [active, setActive] = useState<NavId>("button");

  const currentPage = NAV_ITEMS.find((item) => item.id === active)?.page;

  return (
    <div className="h-screen flex flex-col bg-bg">
      {/* Header */}
      <header className="shrink-0 border-b border-border px-6 py-3 flex items-center gap-3">
        <div className="w-3 h-3 rounded-full bg-primary" />
        <span className="font-medium text-fg">Ora UI</span>
        <span className="text-fg-secondary text-sm">Component Showcase</span>
        <div className="ml-auto">
          <ThemeToggle />
        </div>
      </header>

      <div className="flex flex-1 overflow-hidden">
        {/* Sidebar */}
        <nav className="w-48 shrink-0 border-r border-border flex flex-col items-center gap-0.5 overflow-y-auto py-2">
            {NAV_ITEMS.map((item) => (
            <Button
              key={item.id}
              variant="ghost"
              size="md"
              className={
                active === item.id
                  ? "bg-bg-subtle text-fg font-medium"
                  : "text-fg-secondary"
              }
              onClick={() => setActive(item.id)}
            >
              {item.label}
            </Button>
          ))}
        </nav>

        {/* Main content */}
        <main className="flex-1 overflow-y-auto">
          <div className="max-w-3xl mx-auto px-8 py-10">{currentPage}</div>
        </main>
      </div>
    </div>
  );
}
