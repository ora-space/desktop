import { Button } from "@ora/ui";
import { Input } from "@ora/ui";

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="mb-12">
      <h2 className="text-xs font-semibold uppercase tracking-widest text-fg-secondary mb-4 pb-2 border-b border-border">
        {title}
      </h2>
      {children}
    </section>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-6 py-3 border-b border-border-subtle last:border-0">
      <span className="w-28 shrink-0 text-xs text-fg-secondary">{label}</span>
      <div className="flex items-center gap-3 flex-wrap">{children}</div>
    </div>
  );
}

export default function App() {
  return (
    <div className="min-h-screen bg-bg">
      {/* Header */}
      <header className="border-b border-border px-8 py-4 flex items-center gap-3">
        <div className="w-3 h-3 rounded-full bg-primary" />
        <span className="font-medium text-fg">Ora UI</span>
        <span className="text-fg-secondary text-sm">Component Showcase</span>
      </header>

      <div className="max-w-3xl mx-auto px-8 py-10">
        {/* Button */}
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
            <Button variant="secondary" disabled>Secondary</Button>
            <Button variant="ghost" disabled>Ghost</Button>
          </Row>
          <Row label="asChild">
            <Button asChild>
              <a href="#">Link Button</a>
            </Button>
          </Row>
        </Section>

        {/* Input */}
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
      </div>
    </div>
  );
}
