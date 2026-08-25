import { useState } from "react";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@ora/ui";
import { RemoteContractError, type SkillImportSession } from "@ora/contracts";
import { PlatformProvider } from "../../platform";
import { createChatStore } from "@ora/chat";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import {
  RolesSettings,
  SkillImportDialog,
  SkillsSettings,
} from "./atoms-settings";

function renderSettings(
  kind: "agent" | "skill",
  configure?: (client: ReturnType<typeof createMockClient>) => void,
) {
  const state = createMockClientState();
  if (kind === "agent") {
    state.agents = [
      {
        id: "agent-1",
        namespace: "local",
        name: "review-agent",
        description: "Reviews changes",
      },
    ];
  } else {
    state.skills = [
      {
        id: "skill-1",
        namespace: "local",
        name: "review-skill",
        description: "Reviews changes",
        source: { kind: "local" } as const,
        availability: "available",
      },
    ];
  }
  const client = createMockClient(state);
  client.agent.get = async ({ agentId }) => ({
    agent: {
      ...state.agents.find((agent) => agent.id === agentId)!,
      content: "**Agent instructions**",
    },
  });
  client.skill.get = async ({ skillId }) => ({
    skill: {
      ...state.skills.find((skill) => skill.id === skillId)!,
      content: "## Skill instructions",
    },
  });
  configure?.(client);
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );

  return render(
    <Wrapper>
      <AppI18nProvider>
        <PlatformProvider adapter={createStubPlatform()}>
          <TooltipProvider>
            {kind === "agent" ? <RolesSettings /> : <SkillsSettings />}
          </TooltipProvider>
        </PlatformProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
}

describe("atom settings content", () => {
  it("loads and updates editable Agent content", async () => {
    const user = userEvent.setup();
    const update = vi.fn(async () => ({
      agent: {
        id: "agent-1",
        namespace: "local",
        name: "review-agent",
        description: "Reviews changes",
      },
    }));
    renderSettings("agent", (client) => {
      client.agent.update = update;
    });

    await user.click(await screen.findByRole("button", { name: "编辑" }));

    const content = await screen.findByLabelText("内容");
    expect(content).toHaveValue("**Agent instructions**");
    expect(content).toBeEnabled();
    await user.clear(content);
    await user.type(content, "# Updated agent");
    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(update).toHaveBeenCalledWith({
        agentId: "agent-1",
        name: "review-agent",
        description: "Reviews changes",
        content: "# Updated agent",
      }),
    );
  });

  it("labels disabled plugin Skills with their source and status and hides mutation actions", async () => {
    renderSettings("skill", (client) => {
      client.skill.list = async () => ({
        skills: [
          {
            id: "plugin:official/review-pack:review",
            namespace: "official/review-pack",
            name: "review",
            description: "Reviews changes",
            source: { kind: "plugin", pluginId: "official/review-pack" },
            availability: "available",
          },
        ],
      });
      client.plugin.listInstalled = async () => ({
        plugins: [
          {
            id: "official/review-pack",
            namespace: "official",
            name: "review-pack",
            description: "Review skills",
            homepage: null,
            license: null,
            displayName: "Review pack",
            version: "1.0.0",
            kind: "skill",
            enabled: false,
            logo: null,
            runtime: "stopped",
          },
        ],
      });
    });

    const item = await screen.findByRole("listitem");
    const source = within(item).getByText("official/review-pack");
    expect(source).toBeVisible();
    expect(
      source.closest('[data-slot="badge"]')?.querySelector("svg"),
    ).not.toBeNull();
    expect(await within(item).findByText("已禁用")).toBeVisible();
    expect(within(item).queryByText(/来自/)).toBeNull();
    expect(within(item).queryByRole("button", { name: "编辑" })).toBeNull();
    expect(within(item).queryByRole("button", { name: "删除" })).toBeNull();
  });
  it("collapses long plugin sources to an icon and reveals the full id on hover", async () => {
    const user = userEvent.setup();
    const pluginId = "official/review-pack-with-a-name-that-does-not-fit";
    renderSettings("skill", (client) => {
      client.skill.list = async () => ({
        skills: [
          {
            id: "plugin:" + pluginId + ":review",
            namespace: pluginId,
            name: "review",
            description: "Reviews changes",
            source: { kind: "plugin", pluginId },
            availability: "available",
          },
        ],
      });
      client.plugin.listInstalled = async () => ({ plugins: [] });
    });

    const item = await screen.findByRole("listitem");
    const sourceIcon = within(item).getByRole("button", { name: pluginId });
    expect(within(item).queryByText(pluginId)).toBeNull();

    await user.hover(sourceIcon);
    const tooltip = await screen.findByText(pluginId);
    expect(tooltip).toBeVisible();
    expect(tooltip).toHaveClass(
      "max-w-64",
      "whitespace-normal",
      "break-all",
      "text-left",
    );
  });

  it("loads and clears editable Skill content", async () => {
    const user = userEvent.setup();
    const update = vi.fn(async () => ({
      skill: {
        id: "skill-1",
        namespace: "local",
        name: "review-skill",
        description: "Reviews changes",
        source: { kind: "local" } as const,
        availability: "available" as const,
      },
    }));
    renderSettings("skill", (client) => {
      client.skill.update = update;
    });

    const importButton = await screen.findByRole("button", {
      name: "导入 Skill",
    });
    const newButton = screen.getByRole("button", { name: "新建 Skill" });
    const skillList = screen.getByRole("list", { name: "Skills" });
    expect(importButton).toHaveClass("border");
    expect(newButton).toHaveClass("border");
    expect(skillList).toHaveClass("md:grid-cols-2");
    expect(await within(skillList).findAllByRole("listitem")).toHaveLength(1);

    await user.click(await screen.findByRole("button", { name: "编辑" }));
    const content = await screen.findByLabelText("内容");
    expect(content).toHaveValue("## Skill instructions");
    await user.clear(content);
    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(update).toHaveBeenCalledWith({
        skillId: "skill-1",
        name: "review-skill",
        description: "Reviews changes",
        content: "",
      }),
    );
  });

  it.each([
    [
      "agent",
      "新建 Role",
      "标题",
      "new-role",
      "Role description",
      "# Role body",
    ],
    [
      "skill",
      "新建 Skill",
      "名称",
      "new-skill",
      "Skill description",
      "# Skill body",
    ],
  ] as const)(
    "creates %s content from the shared editor",
    async (kind, buttonName, nameLabel, name, description, content) => {
      const user = userEvent.setup();
      const createAgent = vi.fn(
        async (request: {
          name: string;
          description: string;
          content?: string;
        }) => ({
          agent: {
            id: "agent-new",
            namespace: "local",
            name: request.name,
            description: request.description,
          },
        }),
      );
      const createSkill = vi.fn(
        async (request: {
          name: string;
          description: string;
          content?: string;
        }) => ({
          skill: {
            id: "skill-new",
            namespace: "local",
            name: request.name,
            description: request.description,
            source: { kind: "local" } as const,
            availability: "available" as const,
          },
        }),
      );
      const create = kind === "agent" ? createAgent : createSkill;
      renderSettings(kind, (client) => {
        if (kind === "agent") client.agent.create = createAgent;
        else client.skill.create = createSkill;
      });

      await user.click(await screen.findByRole("button", { name: buttonName }));
      await user.type(screen.getByLabelText(nameLabel), name);
      await user.type(screen.getByLabelText("描述"), description);
      const contentInput = screen.getByLabelText("内容");
      expect(contentInput).toHaveValue("");
      await user.type(contentInput, content);
      await user.click(screen.getByRole("button", { name: "保存" }));

      await waitFor(() =>
        expect(create).toHaveBeenCalledWith({ name, description, content }),
      );
    },
  );

  it("shows the Role name conflict returned by the backend", async () => {
    const user = userEvent.setup();
    renderSettings("agent", (client) => {
      client.agent.create = async () => {
        throw new RemoteContractError(
          {
            code: "agent_name_conflict",
            params: {},
            requestId: "550e8400-e29b-41d4-a716-446655440000",
          },
          null,
        );
      };
    });

    await user.click(await screen.findByRole("button", { name: "新建 Role" }));
    await user.type(screen.getByLabelText("标题"), "review-agent");
    await user.type(screen.getByLabelText("描述"), "Duplicate");
    await user.click(screen.getByRole("button", { name: "保存" }));

    expect(await screen.findByText("已存在同名 Role。")).toBeInTheDocument();
  });

  it("blocks saving while Agent content is loading or failed", async () => {
    const user = userEvent.setup();
    let rejectLoad: ((reason?: unknown) => void) | undefined;
    renderSettings("agent", (client) => {
      client.agent.get = () =>
        new Promise((_resolve, reject) => {
          rejectLoad = reject;
        });
    });

    await user.click(await screen.findByRole("button", { name: "编辑" }));
    const save = screen.getByRole("button", { name: "保存" });
    expect(screen.getByLabelText("内容")).toBeDisabled();
    expect(save).toBeDisabled();
    rejectLoad?.(new Error("load failed"));
    expect(await screen.findByText("无法加载内容。")).toBeInTheDocument();
    expect(save).toBeDisabled();
  });

  it("offers delete or re-upload when a skill package is unavailable", async () => {
    const user = userEvent.setup();
    const state = createMockClientState();
    state.skills = [
      {
        id: "skill-1",
        name: "review-skill",
        namespace: "local",
        description: "Reviews changes",
        source: { kind: "local" } as const,
        availability: "unavailable",
      },
    ];
    const client = createMockClient(state);
    client.skill.get = async ({ skillId }) => ({
      skill: {
        ...state.skills.find((skill) => skill.id === skillId)!,
        content: "",
      },
    });
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <SkillsSettings />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    expect(await screen.findByText("不可用")).toBeInTheDocument();
    expect(
      screen.getByText("1 个技能的本地文件已丢失，请删除或重新上传。"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "处理" }));
    expect(
      await screen.findByText("“review-skill”的技能包已丢失"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "这个技能还在列表里，但本地文件找不到了。请删除，或重新上传同名技能包。",
      ),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重新上传" }));
    const importDialog = await screen.findByRole("dialog");
    expect(importDialog).toHaveTextContent("导入技能");
    expect(importDialog).toHaveTextContent(
      "请导入名为“review-skill”的技能包以恢复。",
    );
  });

  it("imports one Agent Markdown file with an overwrite decision", async () => {
    const user = userEvent.setup();
    const markdown =
      "---\nname: review-agent\ndescription: Imported review agent\n---\nReview carefully.";
    const prepare = vi.fn(async () => ({
      candidate: {
        name: "review-agent",
        description: "Imported review agent",
        status: "conflict" as const,
        existingAgent: {
          agentId: "agent-1",
          updatedAt: 42,
          description: "Existing agent",
        },
      },
    }));
    const commit = vi.fn(async () => ({
      status: "overwritten" as const,
      agent: {
        id: "agent-1",
        namespace: "local",
        name: "review-agent",
        description: "Imported review agent",
      },
    }));
    renderSettings("agent", (client) => {
      client.agentImport.prepare = prepare;
      client.agentImport.commit = commit;
    });

    const importButton = await screen.findByRole("button", {
      name: "导入 Role",
    });
    expect(importButton).toHaveClass("border");
    await user.click(importButton);

    const dialog = await screen.findByRole("dialog");
    const input = dialog.querySelector<HTMLInputElement>('input[type="file"]');
    expect(input).not.toBeNull();
    expect(input).toHaveAttribute("accept", ".md,text/markdown");
    expect(input).not.toHaveAttribute("multiple");
    const file = new File([markdown], "review-agent.md", {
      type: "text/markdown",
    });
    Object.defineProperty(file, "text", { value: async () => markdown });
    await user.upload(input!, file);

    expect(
      await within(dialog).findByText("Imported review agent"),
    ).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "覆盖" }));
    await user.click(within(dialog).getByRole("button", { name: "确认导入" }));

    await waitFor(() =>
      expect(commit).toHaveBeenCalledWith({
        content: markdown,
        decision: "overwrite",
        expectedAgentId: "agent-1",
        expectedUpdatedAt: 42,
      }),
    );
    expect(prepare).toHaveBeenCalledWith({ content: markdown });
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });

  it("refreshes an Agent conflict when commit reports stale state", async () => {
    const user = userEvent.setup();
    const markdown =
      "---\nname: review-agent\ndescription: Imported review agent\n---\nReview carefully.";
    const prepare = vi
      .fn()
      .mockResolvedValueOnce({
        candidate: {
          name: "review-agent",
          description: "Imported review agent",
          status: "conflict",
          existingAgent: {
            agentId: "agent-1",
            updatedAt: 42,
            description: "Old description",
          },
        },
      })
      .mockResolvedValueOnce({
        candidate: {
          name: "review-agent",
          description: "Imported review agent",
          status: "conflict",
          existingAgent: {
            agentId: "agent-1",
            updatedAt: 43,
            description: "Changed description",
          },
        },
      });
    renderSettings("agent", (client) => {
      client.agentImport.prepare = prepare;
      client.agentImport.commit = async () => ({
        status: "stale_conflict",
        agent: null,
      });
    });

    await user.click(await screen.findByRole("button", { name: "导入 Role" }));
    const dialog = await screen.findByRole("dialog");
    const input = dialog.querySelector<HTMLInputElement>('input[type="file"]')!;
    const file = new File([markdown], "review-agent.md", {
      type: "text/markdown",
    });
    Object.defineProperty(file, "text", { value: async () => markdown });
    await user.upload(input, file);
    await user.click(
      await within(dialog).findByRole("button", { name: "覆盖" }),
    );
    await user.click(within(dialog).getByRole("button", { name: "确认导入" }));

    expect(
      await within(dialog).findByText(
        "目标 Agent 已发生变化，请重新确认处理方式。",
      ),
    ).toBeInTheDocument();
    expect(
      await within(dialog).findByText("现有描述：Changed description"),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByRole("button", { name: "确认导入" }),
    ).toBeDisabled();
    expect(prepare).toHaveBeenCalledTimes(2);
  });

  it("shows a localized reason when a skill import candidate is invalid", () => {
    renderSkillImportDialog({
      sessionId: "import-1",
      status: "prepared",
      createdAt: 1n,
      candidates: [
        {
          candidateId: "candidate-1",
          name: "",
          description: "",
          sourcePath: "broken/SKILL.md",
          fileCount: 1,
          totalSize: 12n,
          status: "invalid",
          errorCode: "name_missing",
          existingSkill: null,
        },
      ],
      progress: { total: 0, processed: 0, results: [] },
    });

    expect(screen.getByText("无效")).toBeInTheDocument();
    expect(screen.getByText("SKILL.md 缺少名称。")).toBeInTheDocument();
  });

  it("shows a localized reason when a skill import result fails", () => {
    renderSkillImportDialog({
      sessionId: "import-1",
      status: "completed",
      createdAt: 1n,
      candidates: [
        {
          candidateId: "candidate-1",
          name: "review",
          description: "Reviews changes",
          sourcePath: "review/SKILL.md",
          fileCount: 1,
          totalSize: 12n,
          status: "ready",
          errorCode: null,
          existingSkill: null,
        },
      ],
      progress: {
        total: 1,
        processed: 1,
        results: [
          {
            candidateId: "candidate-1",
            name: "review",
            status: "failed",
            errorCode: "skill_storage_error",
          },
        ],
      },
    });

    expect(screen.getByText("导入失败")).toBeInTheDocument();
    expect(screen.queryByText("待导入")).not.toBeInTheDocument();
    expect(screen.queryByText(/正在处理/)).not.toBeInTheDocument();
    expect(screen.getByText("无法写入技能文件。")).toBeInTheDocument();
    expect(screen.getByText("1 个技能导入失败。")).toBeInTheDocument();
  });

  it("shows localized importing copy while a skill import is committing", async () => {
    renderSkillImportDialog({
      sessionId: "import-1",
      status: "committing",
      createdAt: 1n,
      candidates: [
        {
          candidateId: "candidate-1",
          name: "review",
          description: "Reviews changes",
          sourcePath: "review/SKILL.md",
          fileCount: 1,
          totalSize: 12n,
          status: "ready",
          errorCode: null,
          existingSkill: null,
        },
      ],
      progress: { total: 1, processed: 0, results: [] },
    });

    expect(await screen.findByText("导入中")).toBeInTheDocument();
    expect(screen.getByText("导入中… 0 / 1")).toBeInTheDocument();
    expect(screen.queryByText("待导入")).not.toBeInTheDocument();
    expect(screen.queryByText("committing")).not.toBeInTheDocument();
  });

  it("shows English import failure copy when the UI language is English", async () => {
    await act(async () => {
      await appI18n.changeLanguage("en-US");
    });
    try {
      renderSkillImportDialog({
        sessionId: "import-1",
        status: "completed",
        createdAt: 1n,
        candidates: [
          {
            candidateId: "candidate-1",
            name: "review",
            description: "Reviews changes",
            sourcePath: "review/SKILL.md",
            fileCount: 1,
            totalSize: 12n,
            status: "ready",
            errorCode: null,
            existingSkill: null,
          },
        ],
        progress: {
          total: 1,
          processed: 1,
          results: [
            {
              candidateId: "candidate-1",
              name: "review",
              status: "failed",
              errorCode: "skill_storage_error",
            },
          ],
        },
      });

      expect(screen.getByText("Import failed")).toBeInTheDocument();
      expect(
        screen.getByText("The skill files could not be written."),
      ).toBeInTheDocument();
      expect(
        screen.getByText("1 skill(s) failed to import."),
      ).toBeInTheDocument();
      expect(screen.queryByText("导入失败")).not.toBeInTheDocument();
      expect(screen.queryByText("无法写入技能文件。")).not.toBeInTheDocument();
    } finally {
      await act(async () => {
        await appI18n.changeLanguage("zh-CN");
      });
    }
  });

  it("blocks confirm when a restore import is missing the target skill", () => {
    renderSkillImportDialog(
      {
        sessionId: "import-1",
        status: "prepared",
        createdAt: 1n,
        candidates: [
          {
            candidateId: "candidate-1",
            name: "other-skill",
            description: "Something else",
            sourcePath: "other-skill/SKILL.md",
            fileCount: 1,
            totalSize: 12n,
            status: "ready",
            errorCode: null,
            existingSkill: null,
          },
        ],
        progress: { total: 0, processed: 0, results: [] },
      },
      { restoreName: "web-tools-guide" },
    );

    expect(
      screen.getByText("请导入名为“web-tools-guide”的技能包以恢复。"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("导入内容里没有名为“web-tools-guide”的技能。"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "确认导入" })).toBeDisabled();
  });

  it("clears restore mode after a successful import and Import another", async () => {
    const user = userEvent.setup();
    renderRestoreImportDialog({
      sessionId: "import-1",
      status: "completed",
      createdAt: 1n,
      candidates: [
        {
          candidateId: "candidate-1",
          name: "web-tools-guide",
          description: "Web tools",
          sourcePath: "SKILL.md",
          fileCount: 5,
          totalSize: 12n,
          status: "ready",
          errorCode: null,
          existingSkill: null,
        },
      ],
      progress: {
        total: 1,
        processed: 1,
        results: [
          {
            candidateId: "candidate-1",
            name: "web-tools-guide",
            status: "imported",
            errorCode: null,
          },
        ],
      },
    });

    expect(await screen.findByText("导入已完成。")).toBeInTheDocument();
    expect(screen.getByText("已导入")).toBeInTheDocument();
    expect(screen.queryByText("待导入")).not.toBeInTheDocument();
    expect(screen.queryByText(/正在处理/)).not.toBeInTheDocument();
    expect(screen.queryByText(/请导入名为/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "继续导入" }));
    expect(
      screen.getByRole("button", { name: "选择文件夹" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "选择一个技能文件夹或 ZIP、.skill、.tar.gz、.tgz 压缩包。导入前会先检查所有候选项。",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/请导入名为/)).not.toBeInTheDocument();
  });
});

/** Opens the skill import dialog on a frozen session so failure copy can be asserted. */
function renderSkillImportDialog(
  session: SkillImportSession,
  extras?: { restoreName?: string },
) {
  const client = createMockClient(createMockClientState());
  client.skillImport.get = async () => ({ session });
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  return render(
    <Wrapper>
      <AppI18nProvider>
        <PlatformProvider adapter={createStubPlatform()}>
          <SkillImportDialog
            open
            onOpenChange={() => undefined}
            onCompleted={() => undefined}
            initialSession={session}
            restoreName={extras?.restoreName}
          />
        </PlatformProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
}

/** Keeps restoreName in React state so successful restore can drop the name constraint. */
function renderRestoreImportDialog(session: SkillImportSession) {
  const client = createMockClient(createMockClientState());
  client.skillImport.get = async () => ({ session });
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );

  function Harness() {
    const [restoreName, setRestoreName] = useState<string | null>(
      "web-tools-guide",
    );
    return (
      <SkillImportDialog
        open
        restoreName={restoreName}
        onClearRestore={() => setRestoreName(null)}
        onOpenChange={() => undefined}
        onCompleted={() => undefined}
        initialSession={session}
      />
    );
  }

  return render(
    <Wrapper>
      <AppI18nProvider>
        <PlatformProvider adapter={createStubPlatform()}>
          <Harness />
        </PlatformProvider>
      </AppI18nProvider>
    </Wrapper>,
  );
}
