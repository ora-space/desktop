import { useState } from "react";
import { Check, ChevronsUpDown } from "lucide-react";
import {
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Popover,
  PopoverContent,
  PopoverTrigger,
  cn,
} from "@ora/ui";
import { Section, Row } from "./shared";

const FRAMEWORKS = [
  { value: "next.js", label: "Next.js" },
  { value: "sveltekit", label: "SvelteKit" },
  { value: "nuxt.js", label: "Nuxt.js" },
  { value: "remix", label: "Remix" },
  { value: "astro", label: "Astro" },
];

const LANGUAGES = [
  { value: "typescript", label: "TypeScript" },
  { value: "javascript", label: "JavaScript" },
  { value: "python", label: "Python" },
  { value: "rust", label: "Rust" },
  { value: "go", label: "Go" },
];

function SingleCombobox({
  options,
  placeholder,
}: {
  options: { value: string; label: string }[];
  placeholder: string;
}) {
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState("");

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="md"
          className="w-48 justify-between font-normal"
          aria-expanded={open}
        >
          {value
            ? options.find((o) => o.value === value)?.label
            : placeholder}
          <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 text-fg-secondary" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-48 p-0">
        <Command>
          <CommandInput placeholder={`Search...`} />
          <CommandList>
            <CommandEmpty>No results found.</CommandEmpty>
            <CommandGroup>
              {options.map((option) => (
                <CommandItem
                  key={option.value}
                  value={option.value}
                  onSelect={(current) => {
                    setValue(current === value ? "" : current);
                    setOpen(false);
                  }}
                >
                  <Check
                    className={cn(
                      "h-3.5 w-3.5",
                      value === option.value ? "opacity-100" : "opacity-0",
                    )}
                  />
                  {option.label}
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

function MultiCombobox() {
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<string[]>([]);

  const toggle = (value: string) => {
    setSelected((prev) =>
      prev.includes(value) ? prev.filter((v) => v !== value) : [...prev, value],
    );
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="md"
          className="w-48 justify-between font-normal"
          aria-expanded={open}
        >
          {selected.length > 0 ? `${selected.length} selected` : "Select languages..."}
          <ChevronsUpDown className="h-3.5 w-3.5 shrink-0 text-fg-secondary" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-48 p-0">
        <Command>
          <CommandInput placeholder="Search..." />
          <CommandList>
            <CommandEmpty>No results found.</CommandEmpty>
            <CommandGroup>
              {LANGUAGES.map((lang) => (
                <CommandItem
                  key={lang.value}
                  value={lang.value}
                  onSelect={toggle}
                >
                  <Check
                    className={cn(
                      "h-3.5 w-3.5",
                      selected.includes(lang.value) ? "opacity-100" : "opacity-0",
                    )}
                  />
                  {lang.label}
                </CommandItem>
              ))}
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

export default function ComboboxPage() {
  return (
    <>
      <Section title="Combobox">
        <Row label="single select">
          <SingleCombobox options={FRAMEWORKS} placeholder="Select framework..." />
        </Row>
        <Row label="multi select">
          <MultiCombobox />
        </Row>
        <Row label="pre-selected">
          <SingleCombobox options={FRAMEWORKS} placeholder="Select framework..." />
        </Row>
      </Section>
    </>
  );
}
