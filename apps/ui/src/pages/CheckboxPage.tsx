import { Checkbox } from "@ora/ui";
import { Section, Row } from "./shared";

export default function CheckboxPage() {
  return (
    <Section title="Checkbox">
      <Row label="default">
        <div className="flex items-center space-x-2 text-fg">
          <Checkbox id="terms" />
          <label
            htmlFor="terms"
            className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
          >
            Accept terms and conditions
          </label>
        </div>
      </Row>
      <Row label="disabled">
        <div className="flex items-center space-x-2 text-fg">
          <Checkbox id="disabled-terms" disabled />
          <label
            htmlFor="disabled-terms"
            className="text-sm font-medium leading-none opacity-50"
          >
            Disabled
          </label>
        </div>
      </Row>
    </Section>
  );
}
