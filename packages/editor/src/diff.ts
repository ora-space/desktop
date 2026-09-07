import isEqual from "lodash/isEqual.js";
import { type JSONContent } from "@tiptap/react";

export type DiffStatus = "added" | "removed" | undefined;

function areBothLists(a: JSONContent, b: JSONContent): boolean {
  const listTypes = ["bulletList", "orderedList", "taskList"];
  return !!a.type && a.type === b.type && listTypes.includes(a.type);
}

/**
 * Apply diff status to a node and its content recursively
 */
function applyDiffStatus(node: JSONContent, status: DiffStatus): JSONContent {
  const result: JSONContent = {
    ...node,
    attrs: {
      ...node.attrs,
      diffStatus: status,
    },
  };
  if (node.content) {
    result.content = node.content.map((child) =>
      applyDiffStatus(child, status),
    );
  }
  return result;
}

/**
 * Compare two TipTap documents and return a merged document with diff status
 * Uses node-level comparison with LCS (Longest Common Subsequence) approach
 */
export function diffJSONContent(
  oldDoc: JSONContent | null,
  newDoc: JSONContent | null,
): JSONContent {
  if (!oldDoc && !newDoc) {
    return { type: "doc", content: [] };
  }
  if (!oldDoc) {
    return {
      type: "doc",
      content: (newDoc?.content || []).map((node) =>
        applyDiffStatus(node, "added"),
      ),
    };
  }
  if (!newDoc) {
    return {
      type: "doc",
      content: (oldDoc?.content || []).map((node) =>
        applyDiffStatus(node, "removed"),
      ),
    };
  }

  const oldNodes = oldDoc.content || [];
  const newNodes = newDoc.content || [];

  // Use LCS-based diff algorithm
  const result = diffNodeArrays(oldNodes, newNodes);

  return {
    type: "doc",
    content: result,
  };
}

/**
 * Diff two arrays of nodes using a modified LCS approach
 * Produces a unified diff view: removed nodes (red), then added nodes (green)
 */
function diffNodeArrays(
  oldNodes: JSONContent[],
  newNodes: JSONContent[],
): JSONContent[] {
  const result: JSONContent[] = [];

  const m = oldNodes.length;
  const n = newNodes.length;

  // Create (m+1) x (n+1) table filled with 0
  const dp = Array.from({ length: m + 1 }, () => Array(n + 1).fill(0));

  for (let i = 1; i <= m; i++) {
    for (let j = 1; j <= n; j++) {
      if (
        isEqual(oldNodes[i - 1], newNodes[j - 1]) ||
        (areBothLists(oldNodes[i - 1], newNodes[j - 1]) &&
          isEqual(oldNodes[i - 1].attrs, newNodes[j - 1].attrs))
      ) {
        dp[i][j] = dp[i - 1][j - 1] + 1; // ↖ match
      } else {
        dp[i][j] = Math.max(dp[i - 1][j], dp[i][j - 1]); // ⬆ or ⬅
      }
    }
  }

  let i = oldNodes.length;
  let j = newNodes.length;
  while (i > 0 || j > 0) {
    // Unchanged
    if (i > 0 && j > 0 && isEqual(oldNodes[i - 1], newNodes[j - 1])) {
      result.push(oldNodes[i - 1]); // ↖
      i--;
      j--;
    }
    // handle lists
    else if (
      i > 0 &&
      j > 0 &&
      areBothLists(oldNodes[i - 1], newNodes[j - 1]) &&
      isEqual(oldNodes[i - 1].attrs, newNodes[j - 1].attrs)
    ) {
      result.push({
        ...oldNodes[i - 1],
        content: diffNodeArrays(
          oldNodes[i - 1].content!,
          newNodes[j - 1].content!,
        ),
      }); // ↖
      i--;
      j--;
    }
    // Added
    else if (j > 0 && (i === 0 || dp[i][j - 1] >= dp[i - 1][j])) {
      result.push(applyDiffStatus(newNodes[j - 1], "added"));
      j--;
    }
    // Removed
    else {
      result.push(applyDiffStatus(oldNodes[i - 1], "removed"));
      i--;
    }
  }

  return result.reverse();
}
