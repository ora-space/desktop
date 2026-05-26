import { Label, RadioGroup, RadioGroupItem } from "@ora/ui";
import { Section, Row } from "./shared";

export default function RadioGroupPage() {
  return (
    <>
      <Section title="Radio Group">
        <Row label="basic">
          <RadioGroup defaultValue="option-2">
            <div className="flex items-center gap-2">
              <RadioGroupItem value="option-1" id="r1" />
              <Label htmlFor="r1">Option One</Label>
            </div>
            <div className="flex items-center gap-2">
              <RadioGroupItem value="option-2" id="r2" />
              <Label htmlFor="r2">Option Two</Label>
            </div>
            <div className="flex items-center gap-2">
              <RadioGroupItem value="option-3" id="r3" />
              <Label htmlFor="r3">Option Three</Label>
            </div>
          </RadioGroup>
        </Row>

        <Row label="horizontal">
          <RadioGroup defaultValue="card" className="flex flex-row gap-4">
            <div className="flex items-center gap-2">
              <RadioGroupItem value="card" id="h-card" />
              <Label htmlFor="h-card">Card</Label>
            </div>
            <div className="flex items-center gap-2">
              <RadioGroupItem value="paypal" id="h-paypal" />
              <Label htmlFor="h-paypal">PayPal</Label>
            </div>
            <div className="flex items-center gap-2">
              <RadioGroupItem value="apple" id="h-apple" />
              <Label htmlFor="h-apple">Apple Pay</Label>
            </div>
          </RadioGroup>
        </Row>

        <Row label="disabled">
          <RadioGroup defaultValue="b">
            <div className="flex items-center gap-2">
              <RadioGroupItem value="a" id="d-a" disabled />
              <Label htmlFor="d-a" className="opacity-50 cursor-not-allowed">Disabled unselected</Label>
            </div>
            <div className="flex items-center gap-2">
              <RadioGroupItem value="b" id="d-b" disabled />
              <Label htmlFor="d-b" className="opacity-50 cursor-not-allowed">Disabled selected</Label>
            </div>
            <div className="flex items-center gap-2">
              <RadioGroupItem value="c" id="d-c" />
              <Label htmlFor="d-c">Enabled</Label>
            </div>
          </RadioGroup>
        </Row>
      </Section>
    </>
  );
}
