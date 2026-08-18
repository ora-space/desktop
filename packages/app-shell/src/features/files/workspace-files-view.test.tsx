import { createElement, type ReactNode } from "react";
import { render, screen } from "@testing-library/react";
import { RemoteContractError } from "@ora/contracts";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { ContractsClientContext } from "../../contracts-client-context";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { WorkspaceFilesView } from "./workspace-files-view";

/** Renders Files with a chat-driven path that the workspace cannot resolve. */
function renderMissingFile() {
  const client = createMockClient(createMockClientState());
  client.fileSystem.readWorkspaceFile = async () => {
    throw new RemoteContractError(
      {
        code: "file_system_path_not_found",
        params: {},
        requestId: "eb093a72-6961-4e9f-966a-3d5187958476",
      },
      404,
      null,
    );
  };
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(
      QueryClientProvider,
      { client: queryClient },
      createElement(
        ContractsClientContext.Provider,
        { value: client },
        createElement(AppI18nProvider, null, children),
      ),
    );
  return render(
    <WorkspaceFilesView
      taskId="f06fdb43-1297-4ba3-9143-a7a95ee85b0b"
      hideHeader
      fileRequest={{ path: "crates/acp/src/lib.rs", requestId: 1 }}
    />,
    { wrapper },
  );
}

describe("WorkspaceFilesView missing files", () => {
  it("shows the localized missing-path copy instead of the raw transport error", async () => {
    renderMissingFile();

    expect(
      await screen.findByText(/所选路径不存在|The selected path was not found/),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Remote Ora request failed/)).toBeNull();
  });
});
