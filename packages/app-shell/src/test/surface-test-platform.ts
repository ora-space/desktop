import { vi } from "vitest";
import type {
  PlatformAdapter,
  SurfaceCapability,
  SurfaceEvent,
} from "../platform";
import { createStubPlatform } from "./stub-platform";

/** A stub platform whose surface capability records calls and lets tests emit host events. */
export function createSurfaceTestPlatform(options: { embedded: boolean }): {
  platform: PlatformAdapter;
  surfaces: { [K in keyof SurfaceCapability]: ReturnType<typeof vi.fn> };
  emit: (event: SurfaceEvent) => void;
} {
  const listeners = new Set<(event: SurfaceEvent) => void>();
  const surfaces = {
    capabilities: vi.fn(async () => ({ embedded: options.embedded })),
    list: vi.fn(async () => []),
    open: vi.fn(
      async (
        definition: { pluginId: string; surfaceId: string },
        target: "embedded" | "windowed",
      ) => ({
        instance: 1,
        pluginId: definition.pluginId,
        surfaceId: definition.surfaceId,
        title: "Skill Hub",
        target,
        state: "open" as const,
      }),
    ),
    close: vi.fn(async () => undefined),
    setBounds: vi.fn(async () => undefined),
    setVisible: vi.fn(async () => undefined),
    popout: vi.fn(async () => undefined),
    dock: vi.fn(async () => undefined),
    reload: vi.fn(async () => undefined),
    onEvent: vi.fn(async (listener: (event: SurfaceEvent) => void) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    }),
  };
  return {
    platform: { ...createStubPlatform(), surfaces },
    surfaces,
    emit: (event) => {
      for (const listener of listeners) listener(event);
    },
  };
}
