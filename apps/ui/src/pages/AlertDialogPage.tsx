import { useState } from "react";
import {
  Button,
  AlertDialog,
  AlertDialogTrigger,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogFooter,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogAction,
  AlertDialogCancel,
} from "@ora/ui";
import { Section, Row } from "./shared";

export default function AlertDialogPage() {
  const [confirmed, setConfirmed] = useState(false);

  return (
    <Section title="Alert Dialog">
      <Row label="destructive">
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button variant="destructive">Delete account</Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Are you absolutely sure?</AlertDialogTitle>
              <AlertDialogDescription>
                This action cannot be undone. This will permanently delete your
                account and remove your data from our servers.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction>Delete account</AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </Row>

      <Row label="with callback">
        <AlertDialog onOpenChange={(open) => !open && setConfirmed(false)}>
          <AlertDialogTrigger asChild>
            <Button variant="outline">Confirm action</Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Confirm this action?</AlertDialogTitle>
              <AlertDialogDescription>
                This will apply the changes to your workspace immediately.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction
                className="bg-btn-bg text-btn-fg hover:bg-primary"
                onClick={() => setConfirmed(true)}
              >
                Continue
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
        {confirmed && (
          <span className="text-sm text-fg-secondary">Action confirmed!</span>
        )}
      </Row>
    </Section>
  );
}
