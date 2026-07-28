import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
  Button,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@ora/ui";
import { IconCheck, IconChevronDown } from "@tabler/icons-react";
import { useSettingsStore } from "../../state/stores/settings-store";
import {
  DEFAULT_MOCK_MODEL_ID,
  MOCK_MODEL_PROVIDERS,
  isMockModelId,
  selectedMockModel,
  type MockModelCapability,
} from "./mock-model-catalog";
import { ModelProviderLogo, ProviderLogo } from "./provider-logos";

const CAPABILITY_LABEL_KEYS: Record<MockModelCapability, string> = {
  recommended: "chat.modelSelector.capability.recommended",
  coding: "chat.modelSelector.capability.coding",
  reasoning: "chat.modelSelector.capability.reasoning",
  balanced: "chat.modelSelector.capability.balanced",
  fast: "chat.modelSelector.capability.fast",
  free: "chat.modelSelector.capability.free",
  longContext: "chat.modelSelector.capability.longContext",
};

/**
 * Presents a compact Codex-style mock catalog while the actual ACP session
 * continues to use the user's OpenCode configuration and default provider.
 */
export function ModelSelector({ disabled = false }: { disabled?: boolean }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const persistedModelId = useSettingsStore((state) => state.settings.model);
  const updateSettings = useSettingsStore((state) => state.updateSettings);
  const [displayModelId, setDisplayModelId] = useState(DEFAULT_MOCK_MODEL_ID);
  const selection = selectedMockModel(displayModelId);
  const [expandedProvider, setExpandedProvider] = useState<string | null>(
    selection.provider.id,
  );

  useEffect(() => {
    // Removes only fake ids left by the previous implementation. Runtime model
    // selection remains owned by OpenCode's own config, not this visual picker.
    if (isMockModelId(persistedModelId)) updateSettings({ model: "" });
  }, [persistedModelId, updateSettings]);

  const selectModel = (nextModelId: string) => {
    setDisplayModelId(nextModelId);
    setOpen(false);
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={disabled}
            aria-label={t("chat.modelSelector.label")}
            className="group/model h-7 max-w-60 gap-1.5 rounded-md px-2 text-xs font-normal text-muted-foreground hover:text-foreground"
          />
        }
      >
        <ProviderLogo agentCli="open_code" className="size-3.5 shrink-0" />
        <span className="grid grid-cols-[0fr] opacity-0 transition-all duration-200 group-hover/model:grid-cols-[1fr] group-hover/model:opacity-100 group-data-popup-open/model:grid-cols-[1fr] group-data-popup-open/model:opacity-100">
          <span className="min-w-0 overflow-hidden whitespace-nowrap">
            OpenCode
          </span>
        </span>
        <span className="min-w-0 truncate text-foreground">
          {selection.model.name}
        </span>
        <IconChevronDown
          className="size-3 shrink-0 opacity-50 transition-transform duration-200 group-data-popup-open/model:rotate-180"
          aria-hidden="true"
        />
      </PopoverTrigger>

      <PopoverContent
        align="end"
        side="top"
        className="max-h-[min(20rem,var(--available-height))] w-64 gap-0 overflow-y-auto p-1"
      >
        <div className="flex items-center justify-between px-2 py-1.5">
          <span className="text-xs font-medium text-foreground">
            {t("chat.modelSelector.catalog")}
          </span>
          <span className="text-[10px] text-muted-foreground">OpenCode</span>
        </div>

        <Accordion
          value={expandedProvider === null ? [] : [expandedProvider]}
          onValueChange={(value) => setExpandedProvider(value.at(-1) ?? null)}
        >
          {MOCK_MODEL_PROVIDERS.map((provider) => (
            <AccordionItem
              key={provider.id}
              value={provider.id}
              className="border-border/60"
            >
              <AccordionTrigger className="min-h-8 items-center rounded-sm px-2 py-1 hover:bg-muted hover:no-underline">
                <span className="flex min-w-0 items-center gap-2">
                  <ModelProviderLogo
                    provider={provider.id}
                    className="size-3.5 shrink-0 text-muted-foreground"
                  />
                  <span className="truncate text-xs font-normal">
                    {provider.name}
                  </span>
                  <span className="text-[10px] font-normal tabular-nums text-muted-foreground">
                    {provider.models.length}
                  </span>
                </span>
              </AccordionTrigger>
              <AccordionContent className="pb-1 pl-5 pr-0 pt-0">
                {provider.models.map((model) => {
                  const selected = model.id === selection.model.id;
                  const capability = model.capabilities[0];
                  return (
                    <Button
                      key={model.id}
                      type="button"
                      variant="ghost"
                      aria-pressed={selected}
                      onClick={() => selectModel(model.id)}
                      className="h-10 w-full justify-start gap-2 rounded-sm px-2 py-1 text-left font-normal aria-pressed:bg-muted"
                    >
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-xs text-foreground">
                          {model.name}
                        </span>
                        <span className="block truncate text-[10px] text-muted-foreground">
                          {t(CAPABILITY_LABEL_KEYS[capability])}
                          <span className="px-1" aria-hidden="true">·</span>
                          {t("chat.modelSelector.contextWindow", {
                            context: model.contextWindow,
                          })}
                        </span>
                      </span>
                      {selected && (
                        <IconCheck
                          className="size-3.5 shrink-0 text-foreground"
                          aria-hidden="true"
                        />
                      )}
                    </Button>
                  );
                })}
              </AccordionContent>
            </AccordionItem>
          ))}
        </Accordion>
      </PopoverContent>
    </Popover>
  );
}
