import { Button } from "@ora/ui";
import { Input } from "@ora/ui";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
} from "@ora/ui";
import { Checkbox } from "@ora/ui";
import { Avatar, AvatarFallback, AvatarImage } from "@ora/ui";
import { Alert, AlertTitle, AlertDescription } from "@ora/ui";
import { Badge } from "@ora/ui";
import { User } from "lucide-react";

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="mb-12">
      <h2 className="text-xs font-semibold uppercase tracking-widest text-fg-secondary mb-4 pb-2 border-b border-border">
        {title}
      </h2>
      {children}
    </section>
  );
}

function Row({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-6 py-3 border-b border-border-subtle last:border-0">
      <span className="w-28 shrink-0 text-xs text-fg-secondary">{label}</span>
      <div className="flex items-center gap-3 flex-wrap">{children}</div>
    </div>
  );
}

export default function App() {
  return (
    <div className="min-h-screen bg-bg">
      {/* Header */}
      <header className="border-b border-border px-8 py-4 flex items-center gap-3">
        <div className="w-3 h-3 rounded-full bg-primary" />
        <span className="font-medium text-fg">Ora UI</span>
        <span className="text-fg-secondary text-sm">Component Showcase</span>
      </header>

      <div className="max-w-3xl mx-auto px-8 py-10">
        {/* Button */}
        <Section title="Button">
          <Row label="variant">
            <Button variant="primary">Primary</Button>
            <Button variant="secondary">Secondary</Button>
            <Button variant="ghost">Ghost</Button>
            <Button variant="outline">Outline</Button>
            <Button variant="destructive">Destructive</Button>
          </Row>
          <Row label="size">
            <Button size="sm">Small</Button>
            <Button size="md">Medium</Button>
            <Button size="lg">Large</Button>
          </Row>
          <Row label="disabled">
            <Button disabled>Primary</Button>
            <Button variant="secondary" disabled>
              Secondary
            </Button>
            <Button variant="ghost" disabled>
              Ghost
            </Button>
          </Row>
          <Row label="asChild">
            <Button asChild>
              <a href="#">Link Button</a>
            </Button>
          </Row>
        </Section>

        {/* Input */}
        <Section title="Input">
          <Row label="default">
            <Input placeholder="Type something…" className="max-w-xs" />
          </Row>
          <Row label="size">
            <Input size="sm" placeholder="Small" className="max-w-[160px]" />
            <Input size="md" placeholder="Medium" className="max-w-[160px]" />
            <Input size="lg" placeholder="Large" className="max-w-[160px]" />
          </Row>
          <Row label="disabled">
            <Input disabled placeholder="Disabled" className="max-w-xs" />
          </Row>
          <Row label="types">
            <Input
              type="password"
              placeholder="Password"
              className="max-w-xs"
            />
            <Input type="search" placeholder="Search…" className="max-w-xs" />
          </Row>
        </Section>

        {/* Card */}
        <Section title="Card">
          <Row label="default">
            <Card className="w-[350px]">
              <CardHeader>
                <CardTitle>Create project</CardTitle>
                <CardDescription>
                  Deploy your new project in one-click.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="grid w-full items-center gap-4">
                  <div className="flex flex-col space-y-1.5">
                    <Input id="name" placeholder="Name of your project" />
                  </div>
                </div>
              </CardContent>
              <CardFooter className="flex justify-between">
                <Button variant="outline">Cancel</Button>
                <Button>Deploy</Button>
              </CardFooter>
            </Card>
          </Row>
        </Section>

        {/* Checkbox */}
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

        {/* Alert */}
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

        {/* Avatar */}
        <Section title="Avatar">
          <Row label="default">
            <Avatar>
              <AvatarImage src="https://github.com/shadcn.png" alt="@shadcn" />
              <AvatarFallback>CN</AvatarFallback>
            </Avatar>
            <Avatar>
              <AvatarFallback>
                <User className="h-5 w-5" />
              </AvatarFallback>
            </Avatar>
          </Row>
        </Section>

        {/* Badge */}
        <Section title="Badge">
          <Row label="variants">
            <Badge>Default</Badge>
            <Badge variant="secondary">Secondary</Badge>
            <Badge variant="outline">Outline</Badge>
            <Badge variant="destructive">Destructive</Badge>
          </Row>
        </Section>
      </div>
    </div>
  );
}
