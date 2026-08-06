import type { FileData } from "react-diff-view";

export type DiffFileTreeNode =
  | {
      kind: "directory";
      name: string;
      path: string;
      children: DiffFileTreeNode[];
    }
  | {
      kind: "file";
      name: string;
      path: string;
      file: FileData;
    };

interface MutableDirectory {
  name: string;
  path: string;
  directories: Map<string, MutableDirectory>;
  files: DiffFileTreeNode[];
}

/** Groups parsed patch files by path while keeping directories before files. */
export function buildDiffFileTree(files: FileData[]): DiffFileTreeNode[] {
  const root: MutableDirectory = {
    name: "",
    path: "",
    directories: new Map(),
    files: [],
  };

  for (const file of files) {
    const path = diffFilePath(file);
    const parts = path.split("/").filter(Boolean);
    const fileName = parts.pop() ?? path;
    let directory = root;

    for (const part of parts) {
      const childPath = directory.path === "" ? part : `${directory.path}/${part}`;
      let child = directory.directories.get(part);
      if (child === undefined) {
        child = {
          name: part,
          path: childPath,
          directories: new Map(),
          files: [],
        };
        directory.directories.set(part, child);
      }
      directory = child;
    }

    directory.files.push({ kind: "file", name: fileName, path, file });
  }

  return finalizeDirectory(root);
}

/** Chooses the user-facing path for added, deleted, and renamed files. */
export function diffFilePath(file: FileData): string {
  return file.type === "delete" ? file.oldPath : file.newPath;
}

/** Filters changed files case-insensitively against their complete display paths. */
export function filterDiffFiles(files: FileData[], filter: string): FileData[] {
  const normalizedFilter = filter.trim().toLocaleLowerCase();
  if (normalizedFilter === "") return files;
  return files.filter((file) =>
    diffFilePath(file).toLocaleLowerCase().includes(normalizedFilter),
  );
}

/** Converts the mutable build structure into stable alphabetically sorted nodes. */
function finalizeDirectory(directory: MutableDirectory): DiffFileTreeNode[] {
  const directories = [...directory.directories.values()]
    .sort((left, right) => left.name.localeCompare(right.name))
    .map((child): DiffFileTreeNode => ({
      kind: "directory",
      name: child.name,
      path: child.path,
      children: finalizeDirectory(child),
    }));
  const files = [...directory.files].sort((left, right) => left.name.localeCompare(right.name));
  return [...directories, ...files];
}
