import { useState } from "react";
import { Collapsible, CollapsibleTrigger, CollapsibleContent } from "@ora/ui";
import { ChevronRight, Folder, FolderOpen, FileText, FileCode, FileJson } from "lucide-react";
import { cn } from "@ora/ui";
import { Section, Row } from "./shared";

type FileNode =
  | { kind: "file"; name: string; ext: string }
  | { kind: "folder"; name: string; children: FileNode[] };

const FILE_TREE: FileNode[] = [
  {
    kind: "folder",
    name: "src",
    children: [
      {
        kind: "folder",
        name: "components",
        children: [
          { kind: "file", name: "Button.tsx", ext: "tsx" },
          { kind: "file", name: "Input.tsx", ext: "tsx" },
          { kind: "file", name: "Card.tsx", ext: "tsx" },
        ],
      },
      {
        kind: "folder",
        name: "pages",
        children: [
          { kind: "file", name: "Home.tsx", ext: "tsx" },
          { kind: "file", name: "Settings.tsx", ext: "tsx" },
        ],
      },
      {
        kind: "folder",
        name: "lib",
        children: [
          { kind: "file", name: "utils.ts", ext: "ts" },
          { kind: "file", name: "theme.css", ext: "css" },
        ],
      },
      { kind: "file", name: "App.tsx", ext: "tsx" },
      { kind: "file", name: "main.tsx", ext: "tsx" },
    ],
  },
  {
    kind: "folder",
    name: "public",
    children: [
      { kind: "file", name: "favicon.ico", ext: "ico" },
      { kind: "file", name: "og-image.png", ext: "png" },
    ],
  },
  { kind: "file", name: "package.json", ext: "json" },
  { kind: "file", name: "tsconfig.json", ext: "json" },
  { kind: "file", name: "README.md", ext: "md" },
];

function fileIcon(ext: string) {
  if (ext === "json") return <FileJson className="h-3.5 w-3.5 shrink-0 text-yellow-500" />;
  if (["ts", "tsx"].includes(ext)) return <FileCode className="h-3.5 w-3.5 shrink-0 text-blue-500" />;
  return <FileText className="h-3.5 w-3.5 shrink-0 text-fg-secondary" />;
}

function FileNodeRow({ node, depth }: { node: FileNode; depth: number }) {
  const [open, setOpen] = useState(depth === 0);
  const indent = depth * 16;

  if (node.kind === "file") {
    return (
      <div
        className="flex items-center gap-1.5 py-0.5 px-2 rounded-sm hover:bg-bg-subtle cursor-default select-none"
        style={{ paddingLeft: `${indent + 8}px` }}
      >
        {fileIcon(node.ext)}
        <span className="text-sm text-fg">{node.name}</span>
      </div>
    );
  }

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger asChild>
        <button
          className="flex w-full items-center gap-1.5 py-0.5 px-2 rounded-sm hover:bg-bg-subtle text-left select-none"
          style={{ paddingLeft: `${indent + 8}px` }}
        >
          <ChevronRight
            className={cn(
              "h-3 w-3 shrink-0 text-fg-secondary transition-transform duration-150",
              open && "rotate-90",
            )}
          />
          {open
            ? <FolderOpen className="h-3.5 w-3.5 shrink-0 text-primary" />
            : <Folder className="h-3.5 w-3.5 shrink-0 text-primary" />
          }
          <span className="text-sm text-fg">{node.name}</span>
        </button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        {node.children.map((child) => (
          <FileNodeRow key={child.name} node={child} depth={depth + 1} />
        ))}
      </CollapsibleContent>
    </Collapsible>
  );
}

export default function CollapsiblePage() {
  return (
    <Section title="Collapsible">
      <Row label="file tree">
        <div className="w-64 rounded-md border border-border bg-bg py-1">
          {FILE_TREE.map((node) => (
            <FileNodeRow key={node.name} node={node} depth={0} />
          ))}
        </div>
      </Row>
    </Section>
  );
}
