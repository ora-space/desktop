import { Skeleton } from "@ora/ui";
import { Section, Row } from "./shared";

export default function SkeletonPage() {
  return (
    <>
      <Section title="Skeleton">
        <Row label="text lines">
          <div className="flex flex-col gap-2 w-64">
            <Skeleton className="h-4 w-full" />
            <Skeleton className="h-4 w-4/5" />
            <Skeleton className="h-4 w-3/5" />
          </div>
        </Row>
        <Row label="avatar + text">
          <div className="flex items-center gap-3">
            <Skeleton className="h-10 w-10 rounded-full" />
            <div className="flex flex-col gap-2">
              <Skeleton className="h-4 w-32" />
              <Skeleton className="h-3 w-24" />
            </div>
          </div>
        </Row>
        <Row label="card">
          <div className="w-64 rounded-md border border-border p-4 flex flex-col gap-3">
            <Skeleton className="h-32 w-full rounded-md" />
            <Skeleton className="h-4 w-3/4" />
            <Skeleton className="h-3 w-full" />
            <Skeleton className="h-3 w-4/5" />
          </div>
        </Row>
        <Row label="button">
          <Skeleton className="h-8 w-24 rounded-md" />
        </Row>
        <Row label="sizes">
          <div className="flex flex-col gap-2">
            <Skeleton className="h-3 w-48" />
            <Skeleton className="h-4 w-48" />
            <Skeleton className="h-5 w-48" />
            <Skeleton className="h-6 w-48" />
          </div>
        </Row>
      </Section>
    </>
  );
}
