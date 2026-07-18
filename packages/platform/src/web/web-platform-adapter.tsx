import type { ContractsClient } from "@ora/contracts";
import { type ReactNode } from "react";
import { renderPlatformHost, type PlatformHostRenderer } from "../platform-host-renderer";
import {
  PathSelectionInProgressError,
  type PlatformAdapter,
  type PlatformLocale,
  type SelectPathOptions,
} from "../types";
import { WebPathPickerHost } from "./web-path-picker-host";

interface ActivePathSelection {
  requestId: number;
  options: SelectPathOptions;
  restoreFocusTo: HTMLElement | null;
  resolve: (path: string | null) => void;
}

export type WebPlatformSnapshot =
  | { kind: "idle" }
  | { kind: "selecting"; requestId: number; options: SelectPathOptions };

/** Coordinates Promise-based platform calls with the React-owned Web path picker dialog. */
export class WebPlatformAdapter implements PlatformAdapter, PlatformHostRenderer {
  private activeSelection: ActivePathSelection | null = null;
  private listeners = new Set<() => void>();
  private nextRequestId = 1;
  private snapshot: WebPlatformSnapshot = { kind: "idle" };

  constructor(readonly client: ContractsClient) {}

  /** Opens one Web path picker and resolves after the host confirms or cancels it. */
  selectPath(options: SelectPathOptions): Promise<string | null> {
    if (this.activeSelection !== null) {
      return Promise.reject(new PathSelectionInProgressError());
    }

    return new Promise((resolve) => {
      const requestId = this.nextRequestId;
      this.nextRequestId += 1;
      this.activeSelection = {
        requestId,
        options,
        restoreFocusTo:
          typeof document !== "undefined" && document.activeElement instanceof HTMLElement
            ? document.activeElement
            : null,
        resolve,
      };
      this.snapshot = { kind: "selecting", requestId, options };
      this.emitChange();
    });
  }

  /** Returns the stable external-store snapshot consumed by PlatformHost. */
  getSnapshot = (): WebPlatformSnapshot => this.snapshot;

  /** Subscribes one React host to selection state changes. */
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  /** Completes only the currently visible request so stale async UI cannot resolve a newer picker. */
  completeSelection(requestId: number, path: string | null): void {
    if (this.activeSelection?.requestId !== requestId) {
      return;
    }

    const { resolve, restoreFocusTo } = this.activeSelection;
    this.activeSelection = null;
    this.snapshot = { kind: "idle" };
    this.emitChange();
    resolve(path);
    queueMicrotask(() => restoreFocusTo?.focus());
  }

  /** Supplies the Web-only dialog host without exposing rendering on the public adapter interface. */
  [renderPlatformHost](locale: PlatformLocale): ReactNode {
    return <WebPathPickerHost adapter={this} locale={locale} />;
  }

  /** Notifies React after replacing the immutable external-store snapshot. */
  private emitChange(): void {
    for (const listener of this.listeners) {
      listener();
    }
  }
}

/** Creates the Web platform adapter around the same contracts client injected into AppShell. */
export function createWebPlatformAdapter(client: ContractsClient): WebPlatformAdapter {
  return new WebPlatformAdapter(client);
}
