import { Toggle } from "@ora/ui";
import { Bold, Italic, Underline, AlignLeft, AlignCenter, AlignRight } from "lucide-react";
import { Section, Row } from "./shared";

export default function TogglePage() {
  return (
    <>
      <Section title="Toggle">
        <Row label="default">
          <Toggle>Bold</Toggle>
        </Row>
        <Row label="pressed">
          <Toggle defaultPressed>Bold</Toggle>
        </Row>
        <Row label="variants">
          <Toggle variant="default">Default</Toggle>
          <Toggle variant="outline">Outline</Toggle>
        </Row>
        <Row label="sizes">
          <Toggle size="sm">Sm</Toggle>
          <Toggle size="md">Md</Toggle>
          <Toggle size="lg">Lg</Toggle>
        </Row>
        <Row label="icon">
          <Toggle size="icon" aria-label="Bold">
            <Bold className="h-4 w-4" />
          </Toggle>
          <Toggle size="icon" aria-label="Italic">
            <Italic className="h-4 w-4" />
          </Toggle>
          <Toggle size="icon" aria-label="Underline">
            <Underline className="h-4 w-4" />
          </Toggle>
        </Row>
        <Row label="outline icon">
          <Toggle variant="outline" size="icon" aria-label="Align left">
            <AlignLeft className="h-4 w-4" />
          </Toggle>
          <Toggle variant="outline" size="icon" aria-label="Align center">
            <AlignCenter className="h-4 w-4" />
          </Toggle>
          <Toggle variant="outline" size="icon" aria-label="Align right">
            <AlignRight className="h-4 w-4" />
          </Toggle>
        </Row>
        <Row label="disabled">
          <Toggle disabled>Disabled</Toggle>
          <Toggle disabled defaultPressed>
            Pressed
          </Toggle>
        </Row>
      </Section>
    </>
  );
}
