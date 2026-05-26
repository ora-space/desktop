import { useState } from "react";
import { Button, ThemeToggle } from "@ora/ui";
import AlertPage from "./pages/AlertPage";
import AlertDialogPage from "./pages/AlertDialogPage";
import AvatarPage from "./pages/AvatarPage";
import BadgePage from "./pages/BadgePage";
import BreadcrumbPage from "./pages/BreadcrumbPage";
import ButtonPage from "./pages/ButtonPage";
import CardPage from "./pages/CardPage";
import CheckboxPage from "./pages/CheckboxPage";
import CollapsiblePage from "./pages/CollapsiblePage";
import ComboboxPage from "./pages/ComboboxPage";
import ContextMenuPage from "./pages/ContextMenuPage";
import DialogPage from "./pages/DialogPage";
import DropdownMenuPage from "./pages/DropdownMenuPage";
import EmptyPage from "./pages/EmptyPage";
import FieldPage from "./pages/FieldPage";
import HoverCardPage from "./pages/HoverCardPage";
import InputPage from "./pages/InputPage";
import LabelPage from "./pages/LabelPage";
import MenubarPage from "./pages/MenubarPage";
import PaginationPage from "./pages/PaginationPage";
import PopoverPage from "./pages/PopoverPage";
import ProgressPage from "./pages/ProgressPage";
import RadioGroupPage from "./pages/RadioGroupPage";
import ResizablePage from "./pages/ResizablePage";
import ScrollAreaPage from "./pages/ScrollAreaPage";
import SelectPage from "./pages/SelectPage";
import SeparatorPage from "./pages/SeparatorPage";
import SkeletonPage from "./pages/SkeletonPage";
import SliderPage from "./pages/SliderPage";
import SpinnerPage from "./pages/SpinnerPage";
import SwitchPage from "./pages/SwitchPage";
import TabsPage from "./pages/TabsPage";
import TextareaPage from "./pages/TextareaPage";
import TogglePage from "./pages/TogglePage";
import ToggleGroupPage from "./pages/ToggleGroupPage";
import TooltipPage from "./pages/TooltipPage";

const NAV_ITEMS = [
  { id: "alert", label: "Alert", page: <AlertPage /> },
  { id: "alert-dialog", label: "Alert Dialog", page: <AlertDialogPage /> },
  { id: "avatar", label: "Avatar", page: <AvatarPage /> },
  { id: "badge", label: "Badge", page: <BadgePage /> },
  { id: "breadcrumb", label: "Breadcrumb", page: <BreadcrumbPage /> },
  { id: "button", label: "Button", page: <ButtonPage /> },
  { id: "card", label: "Card", page: <CardPage /> },
  { id: "checkbox", label: "Checkbox", page: <CheckboxPage /> },
  { id: "collapsible", label: "Collapsible", page: <CollapsiblePage /> },
  { id: "combobox", label: "Combobox", page: <ComboboxPage /> },
  { id: "context-menu", label: "Context Menu", page: <ContextMenuPage /> },
  { id: "dialog", label: "Dialog", page: <DialogPage /> },
  { id: "dropdown-menu", label: "Dropdown Menu", page: <DropdownMenuPage /> },
  { id: "empty", label: "Empty", page: <EmptyPage /> },
  { id: "field", label: "Field", page: <FieldPage /> },
  { id: "hover-card", label: "Hover Card", page: <HoverCardPage /> },
  { id: "input", label: "Input", page: <InputPage /> },
  { id: "label", label: "Label", page: <LabelPage /> },
  { id: "menubar", label: "Menubar", page: <MenubarPage /> },
  { id: "pagination", label: "Pagination", page: <PaginationPage /> },
  { id: "popover", label: "Popover", page: <PopoverPage /> },
  { id: "progress", label: "Progress", page: <ProgressPage /> },
  { id: "radio-group", label: "Radio Group", page: <RadioGroupPage /> },
  { id: "resizable", label: "Resizable", page: <ResizablePage /> },
  { id: "scroll-area", label: "Scroll Area", page: <ScrollAreaPage /> },
  { id: "select", label: "Select", page: <SelectPage /> },
  { id: "separator", label: "Separator", page: <SeparatorPage /> },
  { id: "skeleton", label: "Skeleton", page: <SkeletonPage /> },
  { id: "slider", label: "Slider", page: <SliderPage /> },
  { id: "spinner", label: "Spinner", page: <SpinnerPage /> },
  { id: "switch", label: "Switch", page: <SwitchPage /> },
  { id: "tabs", label: "Tabs", page: <TabsPage /> },
  { id: "textarea", label: "Textarea", page: <TextareaPage /> },
  { id: "toggle", label: "Toggle", page: <TogglePage /> },
  { id: "toggle-group", label: "Toggle Group", page: <ToggleGroupPage /> },
  { id: "tooltip", label: "Tooltip", page: <TooltipPage /> },
] as const;

type NavId = (typeof NAV_ITEMS)[number]["id"];

export default function App() {
  const [active, setActive] = useState<NavId>("alert");

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
