import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createDebouncedStateStorage,
  createSafeJSONStorage,
  DEBOUNCED_PERSIST_MS,
  flushDebouncedPersistStorage,
} from "./debounced-json-storage";

describe("createDebouncedStateStorage", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.useFakeTimers();
  });

  afterEach(() => {
    flushDebouncedPersistStorage();
    vi.useRealTimers();
    window.localStorage.clear();
  });

  it("coalesces rapid setItem calls into one disk write", () => {
    const storage = createDebouncedStateStorage(DEBOUNCED_PERSIST_MS);
    storage.setItem("k", "a");
    storage.setItem("k", "b");
    storage.setItem("k", "c");

    expect(window.localStorage.getItem("k")).toBeNull();
    expect(storage.getItem("k")).toBe("c");

    vi.advanceTimersByTime(DEBOUNCED_PERSIST_MS);
    expect(window.localStorage.getItem("k")).toBe("c");
  });

  it("flushes pending writes on demand and on pagehide", () => {
    const storage = createDebouncedStateStorage(DEBOUNCED_PERSIST_MS);
    storage.setItem("k", "pending");
    expect(window.localStorage.getItem("k")).toBeNull();

    storage.flush();
    expect(window.localStorage.getItem("k")).toBe("pending");

    storage.setItem("k", "again");
    expect(window.localStorage.getItem("k")).toBe("pending");
    window.dispatchEvent(new Event("pagehide"));
    expect(window.localStorage.getItem("k")).toBe("again");
  });

  it("flushes when the document becomes hidden", () => {
    const storage = createDebouncedStateStorage(DEBOUNCED_PERSIST_MS);
    storage.setItem("k", "hidden");
    const visibility = vi
      .spyOn(document, "visibilityState", "get")
      .mockReturnValue("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    expect(window.localStorage.getItem("k")).toBe("hidden");
    visibility.mockRestore();
  });

  it("removeItem drops a pending write and clears disk immediately", () => {
    const storage = createDebouncedStateStorage(DEBOUNCED_PERSIST_MS);
    storage.setItem("k", "gone");
    storage.removeItem("k");
    expect(storage.getItem("k")).toBeNull();
    expect(window.localStorage.getItem("k")).toBeNull();
    vi.advanceTimersByTime(DEBOUNCED_PERSIST_MS);
    expect(window.localStorage.getItem("k")).toBeNull();
  });

  it("keeps a timed write pending when storage fails and retries on flush", () => {
    let storageAvailable = false;
    const backing = new Map<string, string>();
    const target = {
      getItem: (name: string) => backing.get(name) ?? null,
      setItem: (name: string, value: string) => {
        if (!storageAvailable)
          throw new DOMException("full", "QuotaExceededError");
        backing.set(name, value);
      },
      removeItem: (name: string) => backing.delete(name),
    } as unknown as Storage;
    const storage = createDebouncedStateStorage(
      DEBOUNCED_PERSIST_MS,
      () => target,
    );
    storage.setItem("k", "recoverable");

    vi.advanceTimersByTime(DEBOUNCED_PERSIST_MS);
    expect(storage.getItem("k")).toBe("recoverable");
    expect(backing.get("k")).toBeUndefined();

    storageAvailable = true;
    storage.flush();
    expect(backing.get("k")).toBe("recoverable");
  });

  it("continues flushing other keys after one key fails", () => {
    const backing = new Map<string, string>();
    const target = {
      getItem: (name: string) => backing.get(name) ?? null,
      setItem: (name: string, value: string) => {
        if (name === "blocked") {
          throw new DOMException("full", "QuotaExceededError");
        }
        backing.set(name, value);
      },
      removeItem: (name: string) => backing.delete(name),
    } as unknown as Storage;
    const storage = createDebouncedStateStorage(
      DEBOUNCED_PERSIST_MS,
      () => target,
    );
    storage.setItem("blocked", "retry later");
    storage.setItem("healthy", "saved");

    expect(() => storage.flush()).not.toThrow();
    expect(storage.getItem("blocked")).toBe("retry later");
    expect(backing.get("healthy")).toBe("saved");
  });

  it("isolates lifecycle flushers when one storage accessor fails", () => {
    const broken = createDebouncedStateStorage(DEBOUNCED_PERSIST_MS, () => {
      throw new DOMException("denied", "SecurityError");
    });
    const healthy = createDebouncedStateStorage(DEBOUNCED_PERSIST_MS);
    broken.setItem("broken", "pending");
    healthy.setItem("healthy", "saved");

    expect(() => window.dispatchEvent(new Event("pagehide"))).not.toThrow();
    expect(window.localStorage.getItem("healthy")).toBe("saved");
  });
});

describe("createSafeJSONStorage", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  afterEach(() => {
    window.localStorage.clear();
  });

  it("treats corrupt JSON as a cache miss instead of rejecting", async () => {
    window.localStorage.setItem("ora.test", "{not json");
    const storage = createSafeJSONStorage();
    await expect(storage.getItem("ora.test")).resolves.toBeNull();
  });

  it("skips setItem when the serialized payload is unchanged", async () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem");
    const storage = createSafeJSONStorage();
    const payload = { state: { n: 1 }, version: 0 };
    storage.setItem("ora.elide", payload);
    const writesAfterFirst = setItem.mock.calls.filter(
      ([key]) => key === "ora.elide",
    ).length;
    storage.setItem("ora.elide", payload);
    const writesAfterSecond = setItem.mock.calls.filter(
      ([key]) => key === "ora.elide",
    ).length;
    expect(writesAfterSecond).toBe(writesAfterFirst);
    setItem.mockRestore();
  });
});
