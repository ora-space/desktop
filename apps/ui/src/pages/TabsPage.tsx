import { Tabs, TabsContent, TabsList, TabsTrigger } from "@ora/ui";
import { Section, Row } from "./shared";

export default function TabsPage() {
  return (
    <>
      <Section title="Tabs">
        <Row label="default">
          <Tabs defaultValue="account" className="w-80">
            <TabsList>
              <TabsTrigger value="account">Account</TabsTrigger>
              <TabsTrigger value="password">Password</TabsTrigger>
            </TabsList>
            <TabsContent value="account">
              <p className="text-sm text-fg-secondary">Manage your account settings.</p>
            </TabsContent>
            <TabsContent value="password">
              <p className="text-sm text-fg-secondary">Change your password here.</p>
            </TabsContent>
          </Tabs>
        </Row>
        <Row label="three tabs">
          <Tabs defaultValue="tab1" className="w-80">
            <TabsList>
              <TabsTrigger value="tab1">Overview</TabsTrigger>
              <TabsTrigger value="tab2">Analytics</TabsTrigger>
              <TabsTrigger value="tab3">Settings</TabsTrigger>
            </TabsList>
            <TabsContent value="tab1">
              <p className="text-sm text-fg-secondary">Overview content goes here.</p>
            </TabsContent>
            <TabsContent value="tab2">
              <p className="text-sm text-fg-secondary">Analytics content goes here.</p>
            </TabsContent>
            <TabsContent value="tab3">
              <p className="text-sm text-fg-secondary">Settings content goes here.</p>
            </TabsContent>
          </Tabs>
        </Row>
        <Row label="disabled tab">
          <Tabs defaultValue="a" className="w-80">
            <TabsList>
              <TabsTrigger value="a">Active</TabsTrigger>
              <TabsTrigger value="b" disabled>
                Disabled
              </TabsTrigger>
              <TabsTrigger value="c">Other</TabsTrigger>
            </TabsList>
            <TabsContent value="a">
              <p className="text-sm text-fg-secondary">Active tab content.</p>
            </TabsContent>
            <TabsContent value="c">
              <p className="text-sm text-fg-secondary">Other tab content.</p>
            </TabsContent>
          </Tabs>
        </Row>
        <Row label="full width">
          <Tabs defaultValue="code" className="w-80">
            <TabsList className="w-full">
              <TabsTrigger value="code" className="flex-1">Code</TabsTrigger>
              <TabsTrigger value="preview" className="flex-1">Preview</TabsTrigger>
            </TabsList>
            <TabsContent value="code">
              <div className="rounded-md bg-bg-subtle p-3 font-mono text-xs text-fg">
                {"<Button>Click me</Button>"}
              </div>
            </TabsContent>
            <TabsContent value="preview">
              <div className="flex items-center justify-center rounded-md border border-border p-4">
                <button className="rounded-md bg-primary px-3 py-1.5 text-sm text-white">
                  Click me
                </button>
              </div>
            </TabsContent>
          </Tabs>
        </Row>
      </Section>
    </>
  );
}
