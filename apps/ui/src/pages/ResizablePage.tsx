import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@ora/ui";
import { Section } from "./shared";

function DemoLabel({ children }: { children: React.ReactNode }) {
  return <p className="text-xs text-fg-secondary mb-2">{children}</p>;
}

function FileTree() {
  const files = ["index.tsx", "App.tsx", "main.tsx", "utils.ts", "theme.css"];
  return (
    <div className="h-full p-3 overflow-auto">
      <p className="text-xs font-semibold text-fg-secondary uppercase tracking-widest mb-2">Explorer</p>
      {files.map((f) => (
        <div key={f} className="py-0.5 px-2 text-[13px] text-fg-secondary hover:text-fg hover:bg-bg-subtle rounded-sm cursor-default">
          {f}
        </div>
      ))}
    </div>
  );
}

function Editor({ title = "App.tsx" }: { title?: string }) {
  return (
    <div className="h-full flex flex-col">
      <div className="shrink-0 border-b border-border px-4 py-1.5 text-xs text-fg-secondary bg-bg-subtle">
        {title}
      </div>
      <div className="flex-1 p-4 font-mono text-xs text-fg-secondary overflow-auto leading-relaxed">
        <p><span className="text-primary">import</span> {"{"} useState {"}"} <span className="text-primary">from</span> <span className="text-green-600 dark:text-green-400">'react'</span>;</p>
        <p className="mt-2"><span className="text-primary">export default function</span> <span className="text-yellow-600 dark:text-yellow-400">App</span>() {"{"}</p>
        <p className="ml-4"><span className="text-primary">const</span> [count, setCount] = useState(<span className="text-blue-500">0</span>);</p>
        <p className="ml-4 mt-2"><span className="text-primary">return</span> (</p>
        <p className="ml-8">{"<"}<span className="text-red-500">div</span>{">"}</p>
        <p className="ml-12">{"<"}<span className="text-red-500">button</span> onClick={"{"}handleClick{"}"}{">"}</p>
        <p className="ml-16">Count: {"{"}count{"}"}</p>
        <p className="ml-12">{"</"}<span className="text-red-500">button</span>{">"}</p>
        <p className="ml-8">{"</"}<span className="text-red-500">div</span>{">"}</p>
        <p className="ml-4">);</p>
        <p>{"}"}</p>
      </div>
    </div>
  );
}

function Terminal() {
  return (
    <div className="h-full flex flex-col bg-bg">
      <div className="shrink-0 border-b border-border px-4 py-1.5 text-xs text-fg-secondary bg-bg-subtle">
        Terminal
      </div>
      <div className="flex-1 p-3 font-mono text-xs overflow-auto leading-relaxed">
        <p className="text-green-600 dark:text-green-400">$ pnpm dev</p>
        <p className="text-fg-secondary mt-1">VITE v8.0.11 ready in 312ms</p>
        <p className="text-fg-secondary">➜ Local: <span className="text-primary">http://localhost:5173/</span></p>
        <p className="text-fg-secondary mt-2">$ <span className="inline-block w-2 h-3 bg-fg-secondary animate-pulse align-middle" /></p>
      </div>
    </div>
  );
}

function Properties() {
  return (
    <div className="h-full p-3 overflow-auto">
      <p className="text-xs font-semibold text-fg-secondary uppercase tracking-widest mb-3">Properties</p>
      {[["width", "100%"], ["height", "auto"], ["display", "flex"], ["gap", "8px"]].map(([k, v]) => (
        <div key={k} className="flex justify-between py-1 text-xs border-b border-border-subtle last:border-0">
          <span className="text-fg-secondary">{k}</span>
          <span className="text-fg font-mono">{v}</span>
        </div>
      ))}
    </div>
  );
}

export default function ResizablePage() {
  return (
    <Section title="Resizable">
      <DemoLabel>Horizontal — two panes</DemoLabel>
      <div className="h-64 w-full rounded-md border border-border mb-8">
        <ResizablePanelGroup orientation="horizontal">
          <ResizablePanel defaultSize={25} minSize={15}>
            <FileTree />
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize={75}>
            <Editor />
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>

      <DemoLabel>Horizontal — three panes</DemoLabel>
      <div className="h-64 w-full rounded-md border border-border mb-8">
        <ResizablePanelGroup orientation="horizontal">
          <ResizablePanel defaultSize={20} minSize={12}>
            <FileTree />
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize={55} minSize={20}>
            <Editor />
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize={25} minSize={15}>
            <Properties />
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>

      <DemoLabel>Vertical — editor + terminal</DemoLabel>
      <div className="h-80 w-full rounded-md border border-border mb-8">
        <ResizablePanelGroup orientation="vertical">
          <ResizablePanel defaultSize={65} minSize={30}>
            <Editor />
          </ResizablePanel>
          <ResizableHandle withHandle orientation="vertical" />
          <ResizablePanel defaultSize={35} minSize={20}>
            <Terminal />
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>

      <DemoLabel>Nested — IDE layout</DemoLabel>
      <div className="h-96 w-full rounded-md border border-border">
        <ResizablePanelGroup orientation="horizontal">
          <ResizablePanel defaultSize={20} minSize={12}>
            <FileTree />
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize={80}>
            <ResizablePanelGroup orientation="vertical">
              <ResizablePanel defaultSize={65} minSize={30}>
                <Editor />
              </ResizablePanel>
              <ResizableHandle withHandle orientation="vertical" />
              <ResizablePanel defaultSize={35} minSize={20}>
                <Terminal />
              </ResizablePanel>
            </ResizablePanelGroup>
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>
    </Section>
  );
}
