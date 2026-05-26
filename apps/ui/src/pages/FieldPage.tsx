import { Button, Field, FieldDescription, FieldError, FieldLabel, Input } from "@ora/ui";
import { Section, Row } from "./shared";

export default function FieldPage() {
  return (
    <>
      <Section title="Field">
        <Row label="basic">
          <Field className="w-64">
            <FieldLabel>Email address</FieldLabel>
            <Input type="email" placeholder="you@example.com" />
          </Field>
        </Row>

        <Row label="with description">
          <Field className="w-64">
            <FieldLabel>Username</FieldLabel>
            <Input placeholder="acme_user" />
            <FieldDescription>
              This is your public display name. It can be changed later.
            </FieldDescription>
          </Field>
        </Row>

        <Row label="with error">
          <Field className="w-64">
            <FieldLabel>Email address</FieldLabel>
            <Input
              type="email"
              defaultValue="not-an-email"
              className="border-red-500 focus-visible:border-red-500"
            />
            <FieldError>Please enter a valid email address.</FieldError>
          </Field>
        </Row>

        <Row label="required">
          <Field className="w-64">
            <FieldLabel required>Password</FieldLabel>
            <Input type="password" placeholder="••••••••" />
            <FieldDescription>Must be at least 8 characters.</FieldDescription>
          </Field>
        </Row>

        <Row label="disabled">
          <Field className="w-64">
            <FieldLabel>API Key</FieldLabel>
            <Input disabled defaultValue="sk-••••••••••••••••" />
            <FieldDescription>Contact admin to rotate your API key.</FieldDescription>
          </Field>
        </Row>

        <Row label="form example">
          <form className="w-64 flex flex-col gap-4" onSubmit={(e) => e.preventDefault()}>
            <Field>
              <FieldLabel required>Full name</FieldLabel>
              <Input placeholder="Eric Wang" />
            </Field>
            <Field>
              <FieldLabel required>Email</FieldLabel>
              <Input type="email" placeholder="eric@example.com" />
            </Field>
            <Field>
              <FieldLabel>Bio</FieldLabel>
              <Input placeholder="Tell us about yourself..." />
              <FieldDescription>Optional. Max 160 characters.</FieldDescription>
            </Field>
            <Button type="submit" className="self-end">
              Save profile
            </Button>
          </form>
        </Row>
      </Section>
    </>
  );
}
