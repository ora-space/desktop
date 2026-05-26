import { Badge } from "@ora/ui";
import { Section, Row } from "./shared";

export default function BadgePage() {
  return (
    <Section title="Badge">
      <Row label="variants">
        <Badge>Default</Badge>
        <Badge variant="secondary">Secondary</Badge>
        <Badge variant="outline">Outline</Badge>
        <Badge variant="destructive">Destructive</Badge>
      </Row>
    </Section>
  );
}
