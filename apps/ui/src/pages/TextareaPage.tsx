import { Textarea } from "@ora/ui";
import { Section, Row } from "./shared";

export default function TextareaPage() {
  return (
    <>
      <Section title="Textarea">
        <Row label="default">
          <Textarea placeholder="Type something…" className="w-64" />
        </Row>
        <Row label="sizes">
          <div className="flex flex-col gap-2">
            <Textarea size="sm" placeholder="Small" className="w-64" rows={2} />
            <Textarea size="md" placeholder="Medium (default)" className="w-64" rows={2} />
            <Textarea size="lg" placeholder="Large" className="w-64" rows={2} />
          </div>
        </Row>
        <Row label="with value">
          <Textarea
            defaultValue="This textarea already has content that spans multiple lines to show how it looks."
            className="w-64"
            rows={3}
          />
        </Row>
        <Row label="disabled">
          <Textarea
            disabled
            placeholder="Disabled textarea"
            className="w-64"
            rows={2}
          />
        </Row>
        <Row label="readonly">
          <Textarea
            readOnly
            defaultValue="Read-only content that cannot be edited."
            className="w-64"
            rows={2}
          />
        </Row>
        <Row label="no resize">
          <Textarea
            placeholder="Resize disabled"
            className="w-64 resize-none"
            rows={3}
          />
        </Row>
        <Row label="full width">
          <Textarea placeholder="Full width textarea" rows={4} />
        </Row>
      </Section>
    </>
  );
}
