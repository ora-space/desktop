import type { Node as TiptapNode } from "@tiptap/pm/model";
import { NodeSelection } from "@tiptap/pm/state";
import type { Editor } from "@tiptap/react";

/**
 * Parses a shortcut key string into an array of key labels.
 *
 * Ora runs on Windows and Linux only, where TipTap's `mod` modifier is Ctrl, so `mod` is shown
 * under that name and every other key keeps its own spelling.
 */
export const parseShortcutKeys = (props: {
  shortcutKeys: string | undefined;
  delimiter?: string;
  capitalize?: boolean;
}) => {
  const { shortcutKeys, delimiter = "+", capitalize = true } = props;

  if (!shortcutKeys) return [];

  return shortcutKeys
    .split(delimiter)
    .map((key) => key.trim())
    .map((key) => (key.toLowerCase() === "mod" ? "ctrl" : key))
    .map((key) =>
      capitalize ? key.charAt(0).toUpperCase() + key.slice(1) : key,
    );
};

/**
 * Checks if a mark exists in the editor schema
 */
export const isMarkInSchema = (
  markName: string,
  editor: Editor | null,
): boolean => {
  if (!editor?.schema) return false;
  return editor.schema.spec.marks.get(markName) !== undefined;
};

/**
 * Checks if a node exists in the editor schema
 */
export const isNodeInSchema = (
  nodeName: string,
  editor: Editor | null,
): boolean => {
  if (!editor?.schema) return false;
  return editor.schema.spec.nodes.get(nodeName) !== undefined;
};

/**
 * Checks if a value is a valid number (not null, undefined, or NaN)
 */
export function isValidPosition(pos: number | null | undefined): pos is number {
  return typeof pos === "number" && pos >= 0;
}

/**
 * Conditionally joins class names, filtering falsey values.
 */
export function cn(
  ...classes: (string | boolean | undefined | null)[]
): string {
  return classes.filter(Boolean).join(" ");
}

/**
 * Checks if one or more extensions are registered in the Tiptap editor.
 */
export function isExtensionAvailable(
  editor: Editor | null,
  extensionNames: string | string[],
): boolean {
  if (!editor) return false;

  const names = Array.isArray(extensionNames)
    ? extensionNames
    : [extensionNames];

  const found = names.some((name) =>
    editor.extensionManager.extensions.some((ext) => ext.name === name),
  );

  if (!found) {
    console.warn(
      `None of the extensions [${names.join(", ")}] were found in the editor schema. Ensure they are included in the editor configuration.`,
    );
  }

  return found;
}

/**
 * Finds a node at the specified position with error handling
 */
export function findNodeAtPosition(editor: Editor, position: number) {
  try {
    const node = editor.state.doc.nodeAt(position);
    if (!node) {
      console.warn(`No node found at position ${position}`);
      return null;
    }
    return node;
  } catch (error) {
    console.error(`Error getting node at position ${position}:`, error);
    return null;
  }
}

/**
 * Finds the position and instance of a node in the document
 */
export function findNodePosition(props: {
  editor: Editor | null;
  node?: TiptapNode | null;
  nodePos?: number | null;
}): { pos: number; node: TiptapNode } | null {
  const { editor, node, nodePos } = props;

  if (!editor || !editor.state?.doc) return null;

  const hasValidNode = node !== undefined && node !== null;
  const hasValidPos = isValidPosition(nodePos);

  if (!hasValidNode && !hasValidPos) {
    return null;
  }

  if (hasValidNode) {
    let foundPos = -1;
    let foundNode: TiptapNode | null = null;

    editor.state.doc.descendants((currentNode, pos) => {
      if (currentNode === node) {
        foundPos = pos;
        foundNode = currentNode;
        return false;
      }
      return true;
    });

    if (foundPos !== -1 && foundNode !== null) {
      return { pos: foundPos, node: foundNode };
    }
  }

  if (hasValidPos) {
    const nodeAtPos = findNodeAtPosition(editor, nodePos!);
    if (nodeAtPos) {
      return { pos: nodePos!, node: nodeAtPos };
    }
  }

  return null;
}

/**
 * Checks if the current selection in the editor is a node selection of specified types
 */
export function isNodeTypeSelected(
  editor: Editor,
  types: string[] = [],
): boolean {
  if (!editor || !editor.state.selection) return false;

  const { state } = editor;
  const { doc, selection } = state;

  if (selection.empty) return false;

  if (selection instanceof NodeSelection) {
    const node = doc.nodeAt(selection.from);
    return node ? types.includes(node.type.name) : false;
  }

  return false;
}

type ProtocolOptions = {
  /**
   * The protocol scheme to be registered.
   * @default '''
   * @example 'ftp'
   * @example 'git'
   */
  scheme: string;

  /**
   * If enabled, it allows optional slashes after the protocol.
   * @default false
   * @example true
   */
  optionalSlashes?: boolean;
};

type ProtocolConfig = Array<ProtocolOptions | string>;

const ATTR_WHITESPACE =
  // eslint-disable-next-line no-control-regex
  /[\u0000-\u0020\u00A0\u1680\u180E\u2000-\u2029\u205F\u3000]/g;

export function isAllowedUri(
  uri: string | undefined,
  protocols?: ProtocolConfig,
) {
  const allowedProtocols: string[] = [
    "http",
    "https",
    "ftp",
    "ftps",
    "mailto",
    "tel",
    "callto",
    "sms",
    "cid",
    "xmpp",
  ];

  if (protocols) {
    protocols.forEach((protocol) => {
      const nextProtocol =
        typeof protocol === "string" ? protocol : protocol.scheme;

      if (nextProtocol) {
        allowedProtocols.push(nextProtocol);
      }
    });
  }

  return (
    !uri ||
    uri.replace(ATTR_WHITESPACE, "").match(
      new RegExp(
        // eslint-disable-next-line no-useless-escape
        `^(?:(?:${allowedProtocols.join("|")}):|[^a-z]|[a-z0-9+.\-]+(?:[^a-z+.\-:]|$))`,
        "i",
      ),
    )
  );
}

export function sanitizeUrl(
  inputUrl: string,
  baseUrl: string,
  protocols?: ProtocolConfig,
): string {
  try {
    const url = new URL(inputUrl, baseUrl);

    if (isAllowedUri(url.href, protocols)) {
      return url.href;
    }
  } catch {
    // If URL creation fails, it's considered invalid
  }
  return "#";
}
