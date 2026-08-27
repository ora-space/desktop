/**
 * Static MCP catalog for workflow Agent node attachments.
 * Authors pick zero or more entries; runtime session wiring stays out of scope.
 */
export interface McpCatalogEntry {
  id: string;
  name: string;
  description: string;
}

/** Seeded MCP servers available in the workflow designer. */
export const MCP_CATALOG: readonly McpCatalogEntry[] = [
  {
    id: "filesystem",
    name: "Filesystem",
    description:
      "Read and write project files through a sandboxed workspace MCP.",
  },
  {
    id: "github",
    name: "GitHub",
    description:
      "Inspect pull requests, issues, and repository metadata via GitHub MCP.",
  },
  {
    id: "browser",
    name: "Browser",
    description: "Navigate pages and capture snapshots for UI verification.",
  },
  {
    id: "postgres",
    name: "Postgres",
    description: "Query and inspect PostgreSQL schemas and rows.",
  },
  {
    id: "notion",
    name: "Notion",
    description: "Search and update Notion pages used as project knowledge.",
  },
  {
    id: "slack",
    name: "Slack",
    description: "Post updates and read channel context from Slack workspaces.",
  },
];
