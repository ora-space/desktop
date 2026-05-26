import { Checkbox, Input, Label } from "@ora/ui";
import { Section, Row } from "./shared";

export default function LabelPage() {
  return (
    <>
      <Section title="Label">
        <Row label="with input">
          <div className="flex flex-col gap-1.5 w-56">
            <Label htmlFor="email-label-demo">Email address</Label>
            <Input id="email-label-demo" type="email" placeholder="you@example.com" />
          </div>
        </Row>

        <Row label="with checkbox">
          <div className="flex items-center gap-2">
            <Checkbox id="terms-label" />
            <Label htmlFor="terms-label">Accept terms and conditions</Label>
          </div>
        </Row>

        <Row label="checkbox group">
          <div className="flex flex-col gap-2">
            <p className="text-xs font-medium text-fg-secondary mb-1">Notifications</p>
            <div className="flex items-center gap-2">
              <Checkbox id="notify-email" defaultChecked />
              <Label htmlFor="notify-email">Email</Label>
            </div>
            <div className="flex items-center gap-2">
              <Checkbox id="notify-push" defaultChecked />
              <Label htmlFor="notify-push">Push notifications</Label>
            </div>
            <div className="flex items-center gap-2">
              <Checkbox id="notify-sms" />
              <Label htmlFor="notify-sms">SMS</Label>
            </div>
          </div>
        </Row>

        <Row label="disabled">
          <div className="flex items-center gap-2">
            <Checkbox id="disabled-label" disabled />
            <Label htmlFor="disabled-label" className="opacity-50 cursor-not-allowed">
              Disabled option
            </Label>
          </div>
        </Row>
      </Section>
    </>
  );
}
