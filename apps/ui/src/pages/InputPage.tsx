import { Input } from "@ora/ui";
import { Section, Row } from "./shared";

export default function InputPage() {
  return (
    <Section title="Input">
      <Row label="default">
        <Input placeholder="Type something…" className="max-w-xs" />
      </Row>
      <Row label="size">
        <Input size="sm" placeholder="Small" className="max-w-[160px]" />
        <Input size="md" placeholder="Medium" className="max-w-[160px]" />
        <Input size="lg" placeholder="Large" className="max-w-[160px]" />
      </Row>
      <Row label="disabled">
        <Input disabled placeholder="Disabled" className="max-w-xs" />
      </Row>
      <Row label="types">
        <Input type="password" placeholder="Password" className="max-w-xs" />
        <Input type="search" placeholder="Search…" className="max-w-xs" />
      </Row>
    </Section>
  );
}
