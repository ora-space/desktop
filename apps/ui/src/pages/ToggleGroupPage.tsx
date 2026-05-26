import { ToggleGroup, ToggleGroupItem } from "@ora/ui";
import { AlignLeft, AlignCenter, AlignRight, Bold, Italic, Underline } from "lucide-react";
import { Section, Row } from "./shared";

export default function ToggleGroupPage() {
  return (
    <>
      <Section title="Toggle Group">
        <Row label="single">
          <ToggleGroup type="single" defaultValue="center">
            <ToggleGroupItem value="left" aria-label="Align left">
              <AlignLeft className="h-4 w-4" />
            </ToggleGroupItem>
            <ToggleGroupItem value="center" aria-label="Align center">
              <AlignCenter className="h-4 w-4" />
            </ToggleGroupItem>
            <ToggleGroupItem value="right" aria-label="Align right">
              <AlignRight className="h-4 w-4" />
            </ToggleGroupItem>
          </ToggleGroup>
        </Row>
        <Row label="multiple">
          <ToggleGroup type="multiple" defaultValue={["bold"]}>
            <ToggleGroupItem value="bold" aria-label="Bold">
              <Bold className="h-4 w-4" />
            </ToggleGroupItem>
            <ToggleGroupItem value="italic" aria-label="Italic">
              <Italic className="h-4 w-4" />
            </ToggleGroupItem>
            <ToggleGroupItem value="underline" aria-label="Underline">
              <Underline className="h-4 w-4" />
            </ToggleGroupItem>
          </ToggleGroup>
        </Row>
        <Row label="outline variant">
          <ToggleGroup type="single" variant="outline" defaultValue="b">
            <ToggleGroupItem value="a">Option A</ToggleGroupItem>
            <ToggleGroupItem value="b">Option B</ToggleGroupItem>
            <ToggleGroupItem value="c">Option C</ToggleGroupItem>
          </ToggleGroup>
        </Row>
        <Row label="sizes">
          <div className="flex flex-col gap-2">
            <ToggleGroup type="single" size="sm" defaultValue="x">
              <ToggleGroupItem value="x">Small</ToggleGroupItem>
              <ToggleGroupItem value="y">Group</ToggleGroupItem>
            </ToggleGroup>
            <ToggleGroup type="single" size="md" defaultValue="x">
              <ToggleGroupItem value="x">Medium</ToggleGroupItem>
              <ToggleGroupItem value="y">Group</ToggleGroupItem>
            </ToggleGroup>
            <ToggleGroup type="single" size="lg" defaultValue="x">
              <ToggleGroupItem value="x">Large</ToggleGroupItem>
              <ToggleGroupItem value="y">Group</ToggleGroupItem>
            </ToggleGroup>
          </div>
        </Row>
        <Row label="disabled">
          <ToggleGroup type="single" disabled defaultValue="a">
            <ToggleGroupItem value="a">One</ToggleGroupItem>
            <ToggleGroupItem value="b">Two</ToggleGroupItem>
            <ToggleGroupItem value="c">Three</ToggleGroupItem>
          </ToggleGroup>
        </Row>
      </Section>
    </>
  );
}
