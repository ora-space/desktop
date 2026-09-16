import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { TooltipProvider } from "@ora/ui";
import { TreeRowOverflowTooltip } from "./tree-row-overflow-tooltip";

describe("TreeRowOverflowTooltip", () => {
  it("shows the full label after hovering the trigger", async () => {
    const user = userEvent.setup();
    const text = "文件树右键菜单以及一段足够长会被截断的标题";
    render(
      <TooltipProvider>
        <TreeRowOverflowTooltip text={text}>
          <div role="button" tabIndex={0}>
            {text}
          </div>
        </TreeRowOverflowTooltip>
      </TooltipProvider>,
    );

    expect(screen.queryByRole("tooltip", { hidden: true })).toBeNull();
    await user.hover(screen.getByRole("button", { name: text }));
    expect(
      await screen.findByRole("tooltip", { hidden: true }),
    ).toHaveTextContent(text);
  });
});
