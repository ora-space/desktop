import { Alert, AlertTitle, AlertDescription } from "@ora/ui";
import { Section, Row } from "./shared";

export default function AlertPage() {
  return (
    <Section title="Alert">
      <Row label="default">
        <Alert>
          <AlertTitle>Heads up!</AlertTitle>
          <AlertDescription>
            You can add components to your app using the cli.
          </AlertDescription>
        </Alert>
      </Row>
      <Row label="destructive">
        <Alert variant="destructive">
          <AlertTitle>Error</AlertTitle>
          <AlertDescription>
            Your session has expired. Please log in again.
          </AlertDescription>
        </Alert>
      </Row>
    </Section>
  );
}
