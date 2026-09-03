import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { RemoteContractError } from "@ora/contracts";
import { toast } from "@ora/ui";
import { expect, it, vi } from "vitest";
import { PlatformProvider } from "../platform";
import { createStubPlatform } from "../test/stub-platform";
import { AppI18nProvider } from "./i18n";
import { appI18n } from "./i18n-instance";
import { useContractErrorToast } from "./use-contract-error-toast";

// Keep this test worker responsible for initializing the instance used by useTranslation.
void appI18n;

/** Exposes the hook through a real click so React owns the update boundary. */
function ErrorTrigger({ error }: { error: unknown }) {
  const showContractError = useContractErrorToast();
  return <button onClick={() => showContractError(error)}>Trigger</button>;
}

it("downloads logs from an internal-error toast action", async () => {
  const downloadToday = vi.fn(async () => true);
  const errorToast = vi.spyOn(toast, "error").mockImplementation(() => "id");
  const successToast = vi
    .spyOn(toast, "success")
    .mockImplementation(() => "id");
  const error = new RemoteContractError(
    {
      code: "internal_error",
      requestId: "00000000-0000-4000-8000-000000000001",
      params: {},
    },
    {},
  );
  const platform = {
    ...createStubPlatform(),
    diagnosticLogs: { downloadToday },
  };
  render(
    <AppI18nProvider>
      <PlatformProvider adapter={platform}>
        <ErrorTrigger error={error} />
      </PlatformProvider>
    </AppI18nProvider>,
  );

  fireEvent.click(screen.getByRole("button", { name: "Trigger" }));

  expect(errorToast).toHaveBeenCalledWith(
    expect.stringContaining("00000000-0000-4000-8000-000000000001"),
    expect.objectContaining({
      action: expect.objectContaining({ label: "下载日志" }),
    }),
  );
  const options = errorToast.mock.calls[0]?.[1];
  const action = options?.action;
  if (action === null || typeof action !== "object" || !("onClick" in action)) {
    throw new Error("diagnostic toast action is missing");
  }
  action.onClick({} as Parameters<typeof action.onClick>[0]);

  await waitFor(() => expect(downloadToday).toHaveBeenCalledOnce());
  expect(successToast).toHaveBeenCalledWith("今日日志已下载。");
});
