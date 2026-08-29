import { useEffect, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AppShell } from "@ora/app-shell";
import { createChatStore } from "@ora/chat";
import { createContractsClient } from "@ora/contracts";
import { createTauriTransport } from "./tauri-transport";
import { createTauriPlatformAdapter } from "./tauri-platform-adapter";

const client = createContractsClient(createTauriTransport());
const chatStore = createChatStore(client.session);
const platform = createTauriPlatformAdapter();

/** Fades out the html-embedded startup splash once the shell has mounted. */
function dismissSplash(): void {
  const splash = document.getElementById("ora-splash");
  if (splash === null) return;
  splash.classList.add("ora-splash-hidden");
  const remove = () => splash.remove();
  splash.addEventListener("transitionend", remove, { once: true });
}

export default function App() {
  // The shell is only revealed once the backend is ready, so the inline splash
  // (logo) stays up during the whole boot instead of flashing white. Non-Tauri
  // preview builds have no backend handshake, so they render immediately.
  const [ready, setReady] = useState(() => !isTauri());

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    void listen("ora-app-ready", () => setReady(true)).then((fn) => {
      unlisten = fn;
    });
    // Safety valve so a failed bootstrap never leaves a frozen splash up.
    const timer = window.setTimeout(() => setReady(true), 8000);
    return () => {
      window.clearTimeout(timer);
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!ready) return;
    dismissSplash();
    return () => document.getElementById("ora-splash")?.remove();
  }, [ready]);

  if (!ready) return null;
  return <AppShell client={client} chatStore={chatStore} platform={platform} />;
}
