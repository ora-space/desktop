import { useState } from "react";
import { Slider } from "@ora/ui";
import { Section, Row } from "./shared";

export default function SliderPage() {
  const [value, setValue] = useState([40]);
  const [range, setRange] = useState([20, 70]);

  return (
    <>
      <Section title="Slider">
        <Row label="default">
          <Slider defaultValue={[50]} max={100} step={1} className="w-64" />
        </Row>
        <Row label="controlled">
          <div className="flex items-center gap-4">
            <Slider
              value={value}
              onValueChange={setValue}
              max={100}
              step={1}
              className="w-48"
            />
            <span className="text-sm text-fg-secondary w-8">{value[0]}</span>
          </div>
        </Row>
        <Row label="range">
          <div className="flex items-center gap-4">
            <Slider
              value={range}
              onValueChange={setRange}
              max={100}
              step={1}
              className="w-48"
            />
            <span className="text-sm text-fg-secondary text-nowrap">
              {range[0]} – {range[1]}
            </span>
          </div>
        </Row>
        <Row label="steps">
          <Slider defaultValue={[3]} min={0} max={10} step={1} className="w-64" />
        </Row>
        <Row label="disabled">
          <Slider defaultValue={[60]} max={100} step={1} disabled className="w-64" />
        </Row>
        <Row label="min / max">
          <Slider defaultValue={[0]} min={-100} max={100} step={10} className="w-64" />
        </Row>
      </Section>
    </>
  );
}
