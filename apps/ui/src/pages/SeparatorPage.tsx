import { Separator } from "@ora/ui";
import { Section, Row } from "./shared";

export default function SeparatorPage() {
  return (
    <>
      <Section title="Separator">
        <Row label="horizontal">
          <div className="w-64">
            <p className="text-sm text-fg">Above the separator</p>
            <Separator className="my-3" />
            <p className="text-sm text-fg">Below the separator</p>
          </div>
        </Row>
        <Row label="vertical">
          <div className="flex h-8 items-center gap-3">
            <span className="text-sm text-fg">Left</span>
            <Separator orientation="vertical" />
            <span className="text-sm text-fg">Center</span>
            <Separator orientation="vertical" />
            <span className="text-sm text-fg">Right</span>
          </div>
        </Row>
        <Row label="in toolbar">
          <div className="flex h-8 items-center gap-1 rounded-md border border-border px-2">
            <button className="px-2 text-xs text-fg hover:text-fg">File</button>
            <button className="px-2 text-xs text-fg hover:text-fg">Edit</button>
            <Separator orientation="vertical" className="h-4 mx-1" />
            <button className="px-2 text-xs text-fg hover:text-fg">View</button>
          </div>
        </Row>
        <Row label="subtle">
          <div className="w-64">
            <p className="text-sm text-fg">Section one</p>
            <Separator className="my-2 bg-border-subtle" />
            <p className="text-sm text-fg-secondary">Section two</p>
          </div>
        </Row>
      </Section>
    </>
  );
}
