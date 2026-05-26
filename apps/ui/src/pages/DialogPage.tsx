import { useState } from "react";
import {
  Button,
  Input,
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  DialogBody,
  DialogClose,
} from "@ora/ui";
import { Section, Row } from "./shared";

export default function DialogPage() {
  const [open, setOpen] = useState(false);

  return (
    <Section title="Dialog">
      <Row label="basic">
        <Dialog>
          <DialogTrigger asChild>
            <Button variant="outline">Open Dialog</Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Edit Profile</DialogTitle>
              <DialogDescription>
                Make changes to your profile here. Click save when you're done.
              </DialogDescription>
            </DialogHeader>
            <DialogBody>
              <div className="flex flex-col gap-3">
                <div className="flex flex-col gap-1.5">
                  <label className="text-sm font-medium text-fg">Name</label>
                  <Input defaultValue="Eric Wang" />
                </div>
                <div className="flex flex-col gap-1.5">
                  <label className="text-sm font-medium text-fg">Email</label>
                  <Input defaultValue="eric@example.com" type="email" />
                </div>
              </div>
            </DialogBody>
            <DialogFooter>
              <DialogClose asChild>
                <Button variant="outline">Cancel</Button>
              </DialogClose>
              <Button>Save changes</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </Row>

      <Row label="controlled">
        <Button variant="outline" onClick={() => setOpen(true)}>
          Open Controlled
        </Button>
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Controlled Dialog</DialogTitle>
              <DialogDescription>
                This dialog is controlled via external state.
              </DialogDescription>
            </DialogHeader>
            <DialogBody>
              <p className="text-sm text-fg-secondary">
                The open state is managed outside the Dialog component.
              </p>
            </DialogBody>
            <DialogFooter>
              <Button onClick={() => setOpen(false)}>Got it</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </Row>
    </Section>
  );
}
