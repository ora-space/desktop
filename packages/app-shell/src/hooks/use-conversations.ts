import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ChatMessage, Conversation } from "../lib/types";
import { createAssistantReply, createId, createSeedConversations, deriveTitle } from "../lib/mock-data";

/** localStorage key under which the prototype persists its conversation state. */
export const CONVERSATIONS_STORAGE_KEY = "ora.web-client.conversations.v1";
const REPLY_DELAY_MS = 650;

interface PersistedState {
  conversations: Conversation[];
  activeId: string | null;
}

/** Reads persisted state, seeding fresh conversations when storage is empty or corrupt. */
function loadPersisted(now: number): PersistedState {
  if (typeof window === "undefined") return { conversations: createSeedConversations(now), activeId: null };
  try {
    const raw = window.localStorage.getItem(CONVERSATIONS_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as PersistedState;
      if (Array.isArray(parsed.conversations)) return parsed;
    }
  } catch {
    /* Ignore corrupt storage and fall back to seed data. */
  }
  return { conversations: createSeedConversations(now), activeId: null };
}

export interface UseConversations {
  conversations: Conversation[];
  activeId: string | null;
  activeConversation: Conversation | null;
  isResponding: boolean;
  newChat: () => void;
  selectConversation: (id: string) => void;
  sendMessage: (text: string) => void;
  renameConversation: (id: string, title: string) => void;
  removeConversation: (id: string) => void;
}

/**
 * Owns the conversation list and active selection for the prototype, mirroring
 * state to `localStorage` and simulating an assistant reply after a short delay.
 */
export function useConversations(): UseConversations {
  // Load once per mount; refs reset between StrictMode remounts, which is harmless.
  const persistedRef = useRef<PersistedState | null>(null);
  if (persistedRef.current === null) persistedRef.current = loadPersisted(Date.now());

  const [conversations, setConversations] = useState<Conversation[]>(persistedRef.current.conversations);
  const [activeId, setActiveId] = useState<string | null>(persistedRef.current.activeId);
  const [isResponding, setIsResponding] = useState(false);

  // Refs mirror state so stable callbacks and the reply timeout read fresh values.
  const activeIdRef = useRef(activeId);
  const isRespondingRef = useRef(isResponding);
  const replyTimeoutRef = useRef<number | null>(null);

  useEffect(() => {
    activeIdRef.current = activeId;
  }, [activeId]);
  useEffect(() => {
    isRespondingRef.current = isResponding;
  }, [isResponding]);

  // Persist on every change.
  useEffect(() => {
    try {
      window.localStorage.setItem(CONVERSATIONS_STORAGE_KEY, JSON.stringify({ conversations, activeId }));
    } catch {
      /* Storage may be unavailable (private mode); the prototype still works in-memory. */
    }
  }, [conversations, activeId]);

  // Cancel a pending reply if the component unmounts mid-response.
  useEffect(() => {
    return () => {
      if (replyTimeoutRef.current !== null) window.clearTimeout(replyTimeoutRef.current);
    };
  }, []);

  const newChat = useCallback(() => {
    activeIdRef.current = null;
    setActiveId(null);
  }, []);

  const selectConversation = useCallback((id: string) => {
    activeIdRef.current = id;
    setActiveId(id);
  }, []);

  const sendMessage = useCallback((text: string) => {
    const content = text.trim();
    if (!content || isRespondingRef.current) return;

    const now = Date.now();
    const userMessage: ChatMessage = { id: createId(), role: "user", content, createdAt: now };

    const currentActiveId = activeIdRef.current;
    // Capture the id the reply must land in, whether we reuse or create a conversation.
    const targetId = currentActiveId ?? createId();

    if (currentActiveId) {
      setConversations((prev) =>
        prev.map((c) => (c.id === currentActiveId ? { ...c, messages: [...c.messages, userMessage], updatedAt: now } : c)),
      );
    } else {
      const conversation: Conversation = {
        id: targetId,
        title: deriveTitle(content),
        messages: [userMessage],
        createdAt: now,
        updatedAt: now,
      };
      activeIdRef.current = targetId;
      setActiveId(targetId);
      setConversations((prev) => [conversation, ...prev]);
    }

    setIsResponding(true);
    isRespondingRef.current = true;

    replyTimeoutRef.current = window.setTimeout(() => {
      const replyAt = Date.now();
      const assistantMessage: ChatMessage = {
        id: createId(),
        role: "assistant",
        content: createAssistantReply(content),
        createdAt: replyAt,
      };
      setConversations((prev) =>
        prev.map((c) => (c.id === targetId ? { ...c, messages: [...c.messages, assistantMessage], updatedAt: replyAt } : c)),
      );
      setIsResponding(false);
      isRespondingRef.current = false;
      replyTimeoutRef.current = null;
    }, REPLY_DELAY_MS);
  }, []);

  const renameConversation = useCallback((id: string, title: string) => {
    const next = title.trim();
    if (!next) return;
    setConversations((prev) => prev.map((c) => (c.id === id ? { ...c, title: next } : c)));
  }, []);

  const removeConversation = useCallback((id: string) => {
    setConversations((prev) => prev.filter((c) => c.id !== id));
    if (activeIdRef.current === id) {
      activeIdRef.current = null;
      setActiveId(null);
    }
  }, []);

  const activeConversation = useMemo(
    () => conversations.find((c) => c.id === activeId) ?? null,
    [conversations, activeId],
  );

  return {
    conversations,
    activeId,
    activeConversation,
    isResponding,
    newChat,
    selectConversation,
    sendMessage,
    renameConversation,
    removeConversation,
  };
}
