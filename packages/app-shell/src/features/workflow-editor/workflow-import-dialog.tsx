import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type DragEvent,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import {
  IconAlertCircle,
  IconAlertTriangle,
  IconCircleCheck,
  IconCloudUpload,
  IconInfoCircle,
} from "@tabler/icons-react";
import {
  Button,
  Checkbox,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  cn,
} from "@ora/ui";
import type { DemoWorkflow } from "@ora/workflow-mock";
import { useOptionalPlatform } from "../../platform";
import {
  formatWorkflowFileSize,
  isValidPublishVersion,
  summarizeWorkflowTransfer,
  type WorkflowDependency,
  type WorkflowImportFailure,
} from "./workflow-transfer";
import { WorkflowTransferDependencies } from "./workflow-transfer-dependencies";

/** Name and size of the selected file, shown under the dialog title. */
export interface WorkflowImportFileInfo {
  name: string;
  size: number;
}

/**
 * Import is a small state machine: choose a file, then either review a parsed workflow or
 * read why the file was rejected. Nothing is persisted before `onConfirm`.
 */
export type WorkflowImportState =
  | { stage: "pick" }
  | {
      stage: "preview";
      file: WorkflowImportFileInfo;
      workflow: DemoWorkflow;
      suggestedVersion: string | null;
    }
  | {
      stage: "failure";
      file: WorkflowImportFileInfo;
      failure: WorkflowImportFailure;
    };

/** User choices collected before the workflow is created. */
export interface WorkflowImportChoices {
  name: string;
  /** `null` lets the backend mint a clock-derived version. */
  version: string | null;
  publish: boolean;
}

interface WorkflowImportDialogProps {
  state: WorkflowImportState;
  dependencies: readonly WorkflowDependency[];
  busy: boolean;
  error: string | null;
  onFile: (file: File) => void;
  onChooseAnother: () => void;
  onCancel: () => void;
  onConfirm: (choices: WorkflowImportChoices) => void;
}

const ACCEPTED_FILE_TYPES = ".json,application/json";

/**
 * Hosts every import step in one dialog so the user never leaves context: a drop zone, the
 * preview with plugin readiness, or an explanation of why the file was rejected.
 */
export function WorkflowImportDialog({
  state,
  dependencies,
  busy,
  error,
  onFile,
  onChooseAnother,
  onCancel,
  onConfirm,
}: WorkflowImportDialogProps) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) {
          onCancel();
        }
      }}
    >
      <DialogContent className="gap-0 p-0 sm:max-w-[620px]">
        {state.stage === "pick" ? (
          <PickStage onFile={onFile} onCancel={onCancel} />
        ) : state.stage === "failure" ? (
          <FailureStage
            file={state.file}
            failure={state.failure}
            onChooseAnother={onChooseAnother}
            onCancel={onCancel}
          />
        ) : (
          <PreviewStage
            // Remount per file so editable fields start from the new file's values.
            key={`${state.file.name}:${state.file.size}`}
            state={state}
            dependencies={dependencies}
            busy={busy}
            error={error}
            onChooseAnother={onChooseAnother}
            onCancel={onCancel}
            onConfirm={onConfirm}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

/** Title block shared by every stage. */
function Header({
  title,
  description,
  mono = false,
}: {
  title: string;
  description: string;
  mono?: boolean;
}) {
  return (
    <DialogHeader className="gap-1 px-5 pt-[18px] pb-2.5">
      <DialogTitle className="text-[15px] font-semibold tracking-[-0.01em]">
        {title}
      </DialogTitle>
      <DialogDescription
        className={cn(
          "text-[12.5px] text-muted-foreground",
          mono && "font-mono",
        )}
      >
        {description}
      </DialogDescription>
    </DialogHeader>
  );
}

/** Reads the first file from a browser drop, including WebView items that omit `files`. */
function fileFromDataTransfer(dataTransfer: DataTransfer): File | undefined {
  const [listed] = Array.from(dataTransfer.files);
  if (listed !== undefined) {
    return listed;
  }
  for (const item of Array.from(dataTransfer.items)) {
    if (item.kind === "file") {
      const file = item.getAsFile();
      if (file !== null) {
        return file;
      }
    }
  }
  return undefined;
}

/** Drop zone that also opens the native picker, matching the host's file selection. */
function PickStage({
  onFile: selectFile,
  onCancel: cancel,
}: {
  onFile: (file: File) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const platform = useOptionalPlatform();
  const selectFileRef = useRef(selectFile);
  useLayoutEffect(() => {
    selectFileRef.current = selectFile;
  }, [selectFile]);
  const [dragging, setDragging] = useState(false);

  /** Accepts the first dropped file; folders and multi-select are not meaningful here. */
  function drop(event: DragEvent<HTMLElement>): void {
    event.preventDefault();
    event.stopPropagation();
    setDragging(false);
    const file = fileFromDataTransfer(event.dataTransfer);
    if (file !== undefined) {
      selectFile(file);
    }
  }

  /** Marks the zone as a copy target so the OS drop is delivered instead of navigating. */
  function dragOver(event: DragEvent<HTMLElement>): void {
    event.preventDefault();
    event.stopPropagation();
    event.dataTransfer.dropEffect = "copy";
    setDragging(true);
  }

  useEffect(() => {
    // WebView2 only fires drop if the window itself cancels the default navigation.
    function allowWindowDrop(event: globalThis.DragEvent): void {
      event.preventDefault();
    }
    function dropOnWindow(event: globalThis.DragEvent): void {
      event.preventDefault();
      const file =
        event.dataTransfer === null
          ? undefined
          : fileFromDataTransfer(event.dataTransfer);
      if (file !== undefined) {
        selectFileRef.current(file);
      }
    }
    window.addEventListener("dragover", allowWindowDrop);
    window.addEventListener("drop", dropOnWindow);
    return () => {
      window.removeEventListener("dragover", allowWindowDrop);
      window.removeEventListener("drop", dropOnWindow);
    };
  }, []);

  useEffect(() => {
    // Explorer → WebView2 drops arrive as native paths, not HTML5 FileList.
    const subscribe = platform?.subscribeOsFileDrop;
    const readOsTextFile = platform?.readOsTextFile;
    if (subscribe === undefined || readOsTextFile === undefined) {
      return;
    }
    return subscribe((paths) => {
      const [path] = paths;
      if (path === undefined) {
        return;
      }
      void readOsTextFile(path)
        .then((dropped) => {
          selectFileRef.current(
            new File([dropped.content], dropped.name, {
              type: "application/json",
            }),
          );
        })
        .catch(() => {
          selectFileRef.current(
            new File(["{"], "dropped.json", { type: "application/json" }),
          );
        });
    });
  }, [platform]);

  return (
    <>
      <Header
        title={t("settings.workflow.importWorkflow")}
        description={t("settings.workflow.transfer.pickDescription")}
      />
      <div className="px-5 pt-1.5 pb-3.5">
        <label
          onDragEnter={dragOver}
          onDragOver={dragOver}
          onDragLeave={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget as Node)) {
              setDragging(false);
            }
          }}
          onDrop={drop}
          className={cn(
            "grid w-full cursor-pointer justify-items-center gap-1.5 rounded-[11px] border-[1.5px] border-dashed border-border bg-muted px-4 py-[26px] text-center outline-none focus-within:ring-2 focus-within:ring-ring",
            dragging && "border-foreground",
          )}
        >
          <IconCloudUpload
            className="size-[26px] text-muted-foreground"
            stroke={1.6}
          />
          <strong className="text-[13px] font-semibold">
            {t("settings.workflow.transfer.dropZoneTitle")}
          </strong>
          <span className="text-[13px] text-muted-foreground">
            {t("settings.workflow.transfer.dropZoneHint")}
          </span>
          <input
            type="file"
            accept={ACCEPTED_FILE_TYPES}
            className="sr-only"
            aria-label={t("settings.workflow.transfer.dropZoneTitle")}
            onChange={(event) => {
              const [file] = Array.from(event.target.files ?? []);
              event.target.value = "";
              if (file !== undefined) {
                selectFile(file);
              }
            }}
          />
        </label>
      </div>
      <Footer>
        <span className="flex-1" />
        <Button variant="outline" onClick={cancel}>
          {t("common.cancel")}
        </Button>
      </Footer>
    </>
  );
}

/** Explains a rejected file; no workflow has been created at this point. */
function FailureStage({
  file,
  failure,
  onChooseAnother: chooseAnother,
  onCancel: cancel,
}: {
  file: WorkflowImportFileInfo;
  failure: WorkflowImportFailure;
  onChooseAnother: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  const location = failure.reason === "invalidJson" ? failure.location : null;
  const detail =
    failure.reason === "invalidJson" && location !== null
      ? t("settings.workflow.transfer.failure.invalidJsonAt", {
          line: location.line,
          column: location.column,
        })
      : failure.reason === "invalidAgentConfig"
        ? t("settings.workflow.transfer.failure.invalidAgentConfigDetail", {
            detail: failure.detail,
          })
        : t(`settings.workflow.transfer.failure.${failure.reason}Detail`);
  return (
    <>
      <Header
        title={t("settings.workflow.transfer.importFailedTitle")}
        description={`${file.name} · ${formatWorkflowFileSize(file.size)}`}
        mono
      />
      <div className="flex flex-col gap-3.5 px-5 pt-1.5 pb-3.5">
        <Banner tone="error" role="alert">
          <b className="font-semibold">
            {t(`settings.workflow.transfer.failure.${failure.reason}`, {
              kind: failure.reason === "unknownNodeKind" ? failure.kind : "",
              node:
                failure.reason === "invalidAgentConfig"
                  ? failure.nodeTitle
                  : "",
            })}
          </b>
          {detail}
        </Banner>
        {location !== null && (
          <div className="overflow-x-auto rounded-[9px] border border-border bg-muted/40 px-3 py-2.5 font-mono text-[12px]">
            {location.excerpt.map((line) => (
              <div key={line.number} className="whitespace-pre">
                <span className="mr-2.5 text-muted-foreground">
                  {line.number}
                </span>
                {line.number === location.line ? (
                  <>
                    {line.text.slice(0, location.column - 1)}
                    <mark className="rounded-[3px] bg-destructive/10 px-0.5 text-destructive">
                      {line.text.charAt(location.column - 1) || " "}
                    </mark>
                    {line.text.slice(location.column)}
                  </>
                ) : (
                  line.text
                )}
              </div>
            ))}
          </div>
        )}
        <p className="m-0 text-[13px] text-muted-foreground">
          {t("settings.workflow.transfer.failureNote")}
        </p>
      </div>
      <Footer>
        <span className="flex-1" />
        <Button variant="outline" onClick={cancel}>
          {t("settings.workflow.transfer.close")}
        </Button>
        <Button onClick={chooseAnother}>
          {t("settings.workflow.transfer.chooseOtherFile")}
        </Button>
      </Footer>
    </>
  );
}

/** Colored notice used for warnings, confirmations, and errors inside the dialog body. */
function Banner({
  tone,
  role,
  children,
}: {
  tone: "warn" | "info" | "ok" | "error";
  role: "alert" | "status";
  children: ReactNode;
}) {
  const Icon =
    tone === "warn"
      ? IconAlertTriangle
      : tone === "ok"
        ? IconCircleCheck
        : tone === "info"
          ? IconInfoCircle
          : IconAlertCircle;
  return (
    <div
      role={role}
      className={cn(
        "flex gap-2.5 rounded-[9px] px-3 py-2.5 text-[12.5px] leading-normal",
        tone === "warn" && "bg-amber-500/10 text-amber-800 dark:text-amber-300",
        tone === "ok" &&
          "bg-emerald-500/10 text-emerald-800 dark:text-emerald-300",
        tone === "info" && "bg-blue-500/10 text-blue-700 dark:text-blue-300",
        tone === "error" && "bg-destructive/10 text-destructive",
      )}
    >
      <Icon className="mt-0.5 size-[15px] shrink-0" />
      <div>{children}</div>
    </div>
  );
}

/** Footer row with a top rule, matching the dialog's header and body gutters. */
function Footer({ children }: { children: ReactNode }) {
  return (
    <DialogFooter className="mx-0 mb-0 flex-row flex-wrap items-center gap-2 rounded-b-xl border-t border-border bg-transparent px-5 pt-3 pb-4">
      {children}
    </DialogFooter>
  );
}

/** Review step: editable name/version, content summary, and plugin readiness. */
function PreviewStage({
  state,
  dependencies,
  busy,
  error,
  onChooseAnother,
  onCancel,
  onConfirm,
}: {
  state: Extract<WorkflowImportState, { stage: "preview" }>;
  dependencies: readonly WorkflowDependency[];
  busy: boolean;
  error: string | null;
  onChooseAnother: () => void;
  onCancel: () => void;
  onConfirm: (choices: WorkflowImportChoices) => void;
}) {
  const { t } = useTranslation();
  const { workflow, file } = state;
  const [name, setName] = useState(workflow.name.trim());
  const [version, setVersion] = useState(state.suggestedVersion ?? "");
  const [publish, setPublish] = useState(true);
  const missing = dependencies.filter(
    (dependency) => dependency.status === "missing",
  ).length;
  const unavailable = dependencies.filter(
    (dependency) => dependency.status === "unavailable",
  ).length;
  const summary = summarizeWorkflowTransfer(workflow);
  const trimmedVersion = version.trim();
  const versionInvalid =
    publish && trimmedVersion !== "" && !isValidPublishVersion(trimmedVersion);
  const canConfirm = name.trim() !== "" && !versionInvalid && !busy;

  /** Emits trimmed choices; a blank version defers naming to the backend. */
  function confirm(): void {
    if (!canConfirm) {
      return;
    }
    onConfirm({
      name: name.trim(),
      version: trimmedVersion === "" ? null : trimmedVersion,
      publish,
    });
  }

  const summaryItems = [
    [summary.nodeCount, t("settings.workflow.transfer.summaryNodes")],
    [summary.agentCount, t("settings.workflow.transfer.summaryAgents")],
    [
      summary.globalVariableCount,
      t("settings.workflow.transfer.summaryGlobals"),
    ],
    [dependencies.length, t("settings.workflow.transfer.summaryDependencies")],
  ] as const;

  return (
    <>
      <DialogHeader className="gap-1 px-5 pt-[18px] pb-2.5">
        <DialogTitle className="text-[15px] font-semibold tracking-[-0.01em]">
          {t("settings.workflow.transfer.importTitle")}
        </DialogTitle>
        <DialogDescription className="font-mono text-[12.5px] text-muted-foreground">
          {`${file.name} · ${formatWorkflowFileSize(file.size)}`}
        </DialogDescription>
      </DialogHeader>
      <div className="flex max-h-[60vh] min-w-0 flex-col gap-3.5 overflow-y-auto px-5 pt-1.5 pb-3.5">
        <div className="grid gap-3 sm:grid-cols-2">
          <label className="block">
            <span className="mb-1.5 block text-[12px] font-medium">
              {t("settings.workflow.workflowName")}
            </span>
            <Input
              value={name}
              onChange={(event) => setName(event.target.value)}
              disabled={busy}
              className="h-[34px]"
            />
          </label>
          <label className="block">
            <span className="mb-1.5 block text-[12px] font-medium">
              {t("settings.workflow.transfer.publishVersion")}
            </span>
            <Input
              value={version}
              onChange={(event) => setVersion(event.target.value)}
              placeholder={t(
                "settings.workflow.transfer.publishVersionPlaceholder",
              )}
              aria-invalid={versionInvalid}
              disabled={busy || !publish}
              className="h-[34px] font-mono"
            />
          </label>
        </div>
        {workflow.description !== "" && (
          <p className="-mt-1.5 text-[13px] text-muted-foreground">
            {workflow.description}
          </p>
        )}
        <dl className="grid grid-cols-2 overflow-hidden rounded-[10px] border border-border sm:grid-cols-4">
          {summaryItems.map(([value, label], index) => (
            <div
              key={label}
              className={cn(
                "flex flex-col-reverse border-border px-3 py-[9px]",
                index < summaryItems.length - 1 && "sm:border-r",
                index % 2 === 0 && "border-r sm:border-r",
                index < 2 && "border-b sm:border-b-0",
              )}
            >
              <dt className="text-[11.5px] text-muted-foreground">{label}</dt>
              <dd className="text-base font-semibold tabular-nums">{value}</dd>
            </div>
          ))}
        </dl>
        {missing > 0 ? (
          <Banner tone="warn" role="status">
            <b className="font-semibold">
              {t("settings.workflow.transfer.missingWarningTitle", {
                total: missing,
              })}
            </b>
            {t("settings.workflow.transfer.missingWarning")}
          </Banner>
        ) : unavailable > 0 ? (
          <Banner tone="info" role="status">
            {t("settings.workflow.transfer.unavailableWarning", {
              total: unavailable,
            })}
          </Banner>
        ) : dependencies.length > 0 ? (
          <Banner tone="ok" role="status">
            {t("settings.workflow.transfer.allInstalled")}
          </Banner>
        ) : null}
        <WorkflowTransferDependencies
          title={t("settings.workflow.transfer.dependencies")}
          dependencies={dependencies}
          variant="import"
        />
        <label className="flex cursor-pointer items-start gap-2 text-[13px]">
          <Checkbox
            checked={publish}
            onCheckedChange={(checked) => setPublish(checked === true)}
            disabled={busy}
            className="mt-0.5"
          />
          <span>
            {t("settings.workflow.transfer.publishAfterImport")}
            <span className="block text-muted-foreground">
              {t("settings.workflow.transfer.publishAfterImportHint")}
            </span>
          </span>
        </label>
        {versionInvalid && (
          <p role="alert" className="text-[12px] text-destructive">
            {t("settings.workflow.transfer.versionInvalid")}
          </p>
        )}
        {error !== null && (
          <p role="alert" className="text-[12px] text-destructive">
            {error}
          </p>
        )}
      </div>
      <Footer>
        <Button variant="outline" disabled={busy} onClick={onChooseAnother}>
          {t("settings.workflow.transfer.chooseAgain")}
        </Button>
        <span className="flex-1" />
        <Button variant="outline" disabled={busy} onClick={onCancel}>
          {t("common.cancel")}
        </Button>
        <Button disabled={!canConfirm} onClick={confirm}>
          {missing > 0
            ? t("settings.workflow.transfer.importAnyway")
            : t("settings.workflow.transfer.confirmImport")}
        </Button>
      </Footer>
    </>
  );
}
