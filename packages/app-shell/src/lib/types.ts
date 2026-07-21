/** The signed-in user surfaced in the sidebar footer. */
export interface CurrentUser {
  name: string;
  email: string;
}

/** One runtime-provided starter prompt localized without coupling the shell to its source. */
export interface ChatSuggestion {
  id: string;
  text: Record<"zh-CN" | "en-US", string>;
}
