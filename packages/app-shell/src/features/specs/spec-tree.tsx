import { useState } from "react";
import type { SpecDocument } from "@ora/contracts";
import { Button, ScrollArea } from "@ora/ui";
import { IconChevronDown, IconChevronRight, IconFileText, IconFolder } from "@tabler/icons-react";

interface TreeNode {
  name: string;
  path: string;
  children: Map<string, TreeNode>;
  document?: SpecDocument;
}

/** Groups documents by workflow while preserving their complete workspace-relative directory paths. */
export function SpecTree({ documents, selectedPath, onSelect }: {
  documents: SpecDocument[];
  selectedPath: string | null;
  onSelect: (path: string) => void;
}) {
  const groups = groupDocuments(documents);
  return (
    <ScrollArea className="h-full">
      <div className="p-2">
        {groups.map(([label, root]) => (
          <TreeBranch key={label} node={root} label={label} depth={0} selectedPath={selectedPath} onSelect={onSelect} initiallyOpen />
        ))}
      </div>
    </ScrollArea>
  );
}

function TreeBranch({ node, label, depth, selectedPath, onSelect, initiallyOpen = false }: {
  node: TreeNode;
  label?: string;
  depth: number;
  selectedPath: string | null;
  onSelect: (path: string) => void;
  initiallyOpen?: boolean;
}) {
  const [open, setOpen] = useState(initiallyOpen || depth < 2);
  if (node.document) {
    return (
      <Button
        variant={selectedPath === node.document.relativePath ? "secondary" : "ghost"}
        size="sm"
        className="h-7 w-full justify-start gap-1.5 px-1.5 font-normal"
        style={{ paddingLeft: `${depth * 12 + 6}px` }}
        onClick={() => onSelect(node.document!.relativePath)}
      >
        <IconFileText className="size-3.5 shrink-0" />
        <span className="truncate text-xs">{node.name}</span>
      </Button>
    );
  }
  const children = [...node.children.values()].sort((left, right) => {
    const leftFile = left.document === undefined ? 0 : 1;
    const rightFile = right.document === undefined ? 0 : 1;
    return leftFile - rightFile || left.name.localeCompare(right.name);
  });
  return (
    <div>
      <Button
        variant="ghost"
        size="sm"
        className="h-7 w-full justify-start gap-1 px-1.5 font-normal"
        style={{ paddingLeft: `${depth * 12 + 4}px` }}
        onClick={() => setOpen((value) => !value)}
      >
        {open ? <IconChevronDown className="size-3.5" /> : <IconChevronRight className="size-3.5" />}
        <IconFolder className="size-3.5 text-amber-600" />
        <span className="truncate text-xs font-medium">{label ?? node.name}</span>
      </Button>
      {open && children.map((child) => (
        <TreeBranch key={child.path} node={child} depth={depth + 1} selectedPath={selectedPath} onSelect={onSelect} />
      ))}
    </div>
  );
}

function groupDocuments(documents: SpecDocument[]): Array<[string, TreeNode]> {
  const groups = new Map<string, TreeNode>();
  for (const document of documents) {
    const label = document.workflow.kind === "open_spec"
      ? "OpenSpec"
      : document.workflow.kind === "superpowers"
        ? "Superpowers"
        : document.workflow.name;
    let root = groups.get(label);
    if (root === undefined) {
      root = { name: label, path: label, children: new Map() };
      groups.set(label, root);
    }
    let parent = root;
    for (const segment of document.relativePath.split("/")) {
      const path = parent.path === label ? segment : `${parent.path}/${segment}`;
      let child = parent.children.get(segment);
      if (child === undefined) {
        child = { name: segment, path, children: new Map() };
        parent.children.set(segment, child);
      }
      parent = child;
    }
    parent.document = document;
  }
  return [...groups.entries()].sort(([left], [right]) => left.localeCompare(right));
}
