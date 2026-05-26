import { Button, Input, Label, Popover, PopoverContent, PopoverTrigger } from "@ora/ui";
import { Settings2 } from "lucide-react";
import { Section, Row } from "./shared";

export default function PopoverPage() {
  return (
    <>
      <Section title="Popover">
        <Row label="basic">
          <Popover>
            <PopoverTrigger asChild>
              <Button variant="outline">Open popover</Button>
            </PopoverTrigger>
            <PopoverContent>
              <p className="text-[13px] text-fg-secondary">
                This is a simple popover with no interactive content.
              </p>
            </PopoverContent>
          </Popover>
        </Row>

        <Row label="with form">
          <Popover>
            <PopoverTrigger asChild>
              <Button variant="outline" size="md">
                <Settings2 className="h-3.5 w-3.5" />
                Dimensions
              </Button>
            </PopoverTrigger>
            <PopoverContent className="w-64">
              <div className="flex flex-col gap-3">
                <p className="text-sm font-semibold text-fg">Dimensions</p>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="pop-width">Width</Label>
                  <Input id="pop-width" defaultValue="100%" size="sm" />
                </div>
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="pop-height">Max. height</Label>
                  <Input id="pop-height" defaultValue="none" size="sm" />
                </div>
                <Button size="sm" className="self-end">Apply</Button>
              </div>
            </PopoverContent>
          </Popover>
        </Row>

        <Row label="side=right">
          <Popover>
            <PopoverTrigger asChild>
              <Button variant="outline">Right side</Button>
            </PopoverTrigger>
            <PopoverContent side="right" className="w-48">
              <p className="text-xs text-fg-secondary">Opens to the right.</p>
            </PopoverContent>
          </Popover>
        </Row>

        <Row label="side=top">
          <Popover>
            <PopoverTrigger asChild>
              <Button variant="outline">Top side</Button>
            </PopoverTrigger>
            <PopoverContent side="top" className="w-48">
              <p className="text-xs text-fg-secondary">Opens upward.</p>
            </PopoverContent>
          </Popover>
        </Row>
      </Section>
    </>
  );
}
