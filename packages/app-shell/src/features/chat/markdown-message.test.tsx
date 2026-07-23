import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { MarkdownMessage } from "./markdown-message";

/** Renders Markdown with the production translation provider used by code controls. */
function renderMarkdown(content: string) {
  return render(<AppI18nProvider><MarkdownMessage content={content} /></AppI18nProvider>);
}

describe("MarkdownMessage", () => {
  it("renders GitHub-flavored Markdown with semantic elements", () => {
    render(<MarkdownMessage content={"## Result\n\n- one\n- two\n\n| Name | Value |\n| --- | --- |\n| Ora | IDE |\n\n`const ready = true;`"} />);

    expect(screen.getByRole("heading", { level: 2, name: "Result" })).toBeInTheDocument();
    expect(screen.getByRole("list")).toHaveTextContent("one");
    expect(screen.getByRole("table")).toHaveTextContent("Ora");
    expect(screen.getByText("const ready = true;")).toHaveClass("font-mono");
  });

  it("keeps links safe and does not interpret raw HTML", () => {
    render(<MarkdownMessage content={'[Documentation](https://example.com)\n\n<script>alert("unsafe")</script>'} />);

    expect(screen.getByRole("link", { name: "Documentation" })).toHaveAttribute("rel", "noopener noreferrer");
    expect(screen.queryByRole("script")).toBeNull();
    expect(screen.getByText(/<script>/)).toBeInTheDocument();
  });

  it("renders a document wrapped in a Markdown code fence", () => {
    render(<MarkdownMessage content={"```markdown\n# Wrapped result\n\n**Rendered**, not code.\n```"} />);

    expect(screen.getByRole("heading", { level: 1, name: "Wrapped result" })).toBeInTheDocument();
    expect(screen.getByText("Rendered").tagName).toBe("STRONG");
    expect(screen.queryByText(/# Wrapped result/)).toBeNull();
  });

  it("preserves ordinary fenced code blocks", () => {
    render(<MarkdownMessage content={"Example:\n\n```markdown\n# Literal Markdown\n```"} />);

    expect(screen.queryByRole("heading", { name: "Literal Markdown" })).toBeNull();
    expect(screen.getByText("# Literal Markdown")).toBeInTheDocument();
  });

  it("adds VS Code theme colors to known code languages", async () => {
    renderMarkdown("```typescript\nconst answer: number = 42;\n```");

    expect(screen.getByRole("code").closest(".markdown-code-block")).not.toBeNull();
    await waitFor(() => expect(screen.getByText("const")).toHaveClass("shiki-token"));
    expect(screen.getByText("const")).toHaveStyle({ color: "#0000FF" });
    expect(screen.getByText("const")).toHaveStyle({ "--shiki-dark": "#569CD6" });
  });

  it("copies and collapses fenced code without losing its toolbar", async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, "writeText");
    renderMarkdown("```typescript\nconst answer = 42;\n```");

    await user.click(screen.getByRole("button", { name: /复制代码|Copy code/ }));
    expect(writeText).toHaveBeenCalledWith("const answer = 42;");
    expect(screen.getByRole("button", { name: /代码已复制|Code copied/ })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /收起代码|Collapse code/ }));
    expect(screen.queryByRole("code")).toBeNull();
    expect(screen.getByText(/已收起 1 行代码|1 line collapsed/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /展开代码|Expand code/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /代码已复制|Code copied/ })).toBeInTheDocument();
  });

  it("provides controls for fenced code without a language label", () => {
    renderMarkdown("```\nfirst line\nsecond line\n```");

    expect(screen.getByText("text")).toBeInTheDocument();
    expect(screen.getByText(/2 行|2 lines/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /复制代码|Copy code/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /收起代码|Collapse code/ })).toBeInTheDocument();
  });
});
