import { useState } from "react";
import { Switch, Label } from "@ora/ui";
import { Section, Row } from "./shared";

export default function SwitchPage() {
  const [checked, setChecked] = useState(false);

  return (
    <>
      <Section title="Switch">
        <Row label="default (off)">
          <Switch />
        </Row>
        <Row label="default (on)">
          <Switch defaultChecked />
        </Row>
        <Row label="controlled">
          <div className="flex items-center gap-3">
            <Switch checked={checked} onCheckedChange={setChecked} id="controlled" />
            <Label htmlFor="controlled" className="text-sm text-fg">
              {checked ? "On" : "Off"}
            </Label>
          </div>
        </Row>
        <Row label="with label">
          <div className="flex items-center gap-2">
            <Switch id="notifications" />
            <Label htmlFor="notifications">Enable notifications</Label>
          </div>
        </Row>
        <Row label="disabled off">
          <Switch disabled />
        </Row>
        <Row label="disabled on">
          <Switch disabled defaultChecked />
        </Row>
        <Row label="form row">
          <div className="flex w-64 items-center justify-between rounded-md border border-border px-4 py-3">
            <div>
              <p className="text-sm font-medium text-fg">Autosave</p>
              <p className="text-xs text-fg-secondary">Save changes automatically</p>
            </div>
            <Switch defaultChecked />
          </div>
        </Row>
      </Section>
    </>
  );
}
