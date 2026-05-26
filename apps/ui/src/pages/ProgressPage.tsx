import { useEffect, useState } from "react";
import { Progress } from "@ora/ui";
import { Section, Row } from "./shared";

function AnimatedProgress() {
  const [value, setValue] = useState(0);

  useEffect(() => {
    const id = setInterval(() => {
      setValue((v) => (v >= 100 ? 0 : v + 5));
    }, 300);
    return () => clearInterval(id);
  }, []);

  return <Progress value={value} className="w-64" />;
}

export default function ProgressPage() {
  return (
    <>
      <Section title="Progress">
        <Row label="0%">
          <Progress value={0} className="w-64" />
        </Row>
        <Row label="33%">
          <Progress value={33} className="w-64" />
        </Row>
        <Row label="66%">
          <Progress value={66} className="w-64" />
        </Row>
        <Row label="100%">
          <Progress value={100} className="w-64" />
        </Row>
        <Row label="animated">
          <AnimatedProgress />
        </Row>
        <Row label="indeterminate">
          <Progress className="w-64 [&>div]:animate-[progress-indeterminate_1.5s_ease-in-out_infinite]" />
        </Row>
        <Row label="sizes">
          <div className="flex flex-col gap-2 w-64">
            <Progress value={60} className="h-1" />
            <Progress value={60} className="h-2" />
            <Progress value={60} className="h-3" />
            <Progress value={60} className="h-4" />
          </div>
        </Row>
      </Section>
    </>
  );
}
