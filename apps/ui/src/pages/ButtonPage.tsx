import { Button } from "@ora/ui";
import { Section, Row } from "./shared";

export default function ButtonPage() {
  return (
    <Section title="Button">
      <Row label="variant">
        <Button variant="primary">Primary</Button>
        <Button variant="secondary">Secondary</Button>
        <Button variant="ghost">Ghost</Button>
        <Button variant="outline">Outline</Button>
        <Button variant="destructive">Destructive</Button>
      </Row>
      <Row label="size">
        <Button size="sm">Small</Button>
        <Button size="md">Medium</Button>
        <Button size="lg">Large</Button>
      </Row>
      <Row label="disabled">
        <Button disabled>Primary</Button>
        <Button variant="secondary" disabled>
          Secondary
        </Button>
        <Button variant="ghost" disabled>
          Ghost
        </Button>
      </Row>
      <Row label="asChild">
        <Button asChild>
          <a href="#">Link Button</a>
        </Button>
      </Row>
    </Section>
  );
}
