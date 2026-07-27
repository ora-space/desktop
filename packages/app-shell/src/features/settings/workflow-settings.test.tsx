import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { WorkflowSettings } from "./workflow-settings";

describe("WorkflowSettings", () => {
  it("loads the mock graph and exposes a deterministic preview", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    expect(screen.getByText("正在加载 mock 工作流…")).toBeInTheDocument();
    expect(await screen.findByText("代码审查工作流")).toBeInTheDocument();
    expect(screen.getByLabelText("工作流画布")).toBeInTheDocument();

    await user.click(screen.getAllByRole("button", { name: "测试运行" })[0]);

    expect(await screen.findByText("模拟运行成功")).toBeInTheDocument();
    expect(screen.getByText(/发现 2 个建议项，未发现阻塞问题。/)).toBeInTheDocument();
  });

  it("zooms around the pointer with the mouse wheel", async () => {
    render(<WorkflowSettings />);
    const canvas = await screen.findByLabelText("工作流画布");

    expect(screen.getByText("100%")).toBeInTheDocument();
    fireEvent.wheel(canvas, { deltaY: -200, clientX: 240, clientY: 180 });

    await waitFor(() => {
      expect(screen.queryByText("100%")).not.toBeInTheDocument();
    });
  });

  it("pans the board by dragging empty canvas space", async () => {
    render(<WorkflowSettings />);
    const canvas = await screen.findByLabelText("工作流画布");
    canvas.setPointerCapture = () => {};
    const stage = canvas.querySelector<HTMLElement>(".origin-top-left");

    expect(stage?.style.transform).toBe("translate(32px, 32px) scale(1)");
    fireEvent.pointerDown(canvas, { button: 0, clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(canvas, { clientX: 150, clientY: 170, pointerId: 1 });
    fireEvent.pointerUp(canvas, { clientX: 150, clientY: 170, pointerId: 1 });

    await waitFor(() => {
      expect(stage?.style.transform).toBe("translate(82px, 102px) scale(1)");
    });
  });
});
