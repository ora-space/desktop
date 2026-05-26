import { Spinner } from "@ora/ui";
import { Section, Row } from "./shared";

export default function SpinnerPage() {
  return (
    <>
      <Section title="Spinner">
        <Row label="sizes">
          <Spinner size="sm" />
          <Spinner size="md" />
          <Spinner size="lg" />
          <Spinner size="xl" />
        </Row>
        <Row label="default (md)">
          <Spinner />
        </Row>
        <Row label="in button">
          <button className="inline-flex items-center gap-2 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-white disabled:opacity-60" disabled>
            <Spinner size="sm" className="border-white/30 border-t-white" />
            Loading…
          </button>
        </Row>
        <Row label="centered">
          <div className="flex h-20 w-48 items-center justify-center rounded-md border border-border">
            <Spinner size="lg" />
          </div>
        </Row>
      </Section>
    </>
  );
}
