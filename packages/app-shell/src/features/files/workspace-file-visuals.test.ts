import { describe, expect, it } from "vitest";
import { workspaceFileVisual } from "./workspace-file-visuals";

describe("workspaceFileVisual", () => {
  it.each([
    ["src/main.rs", "rust", "RUST"],
    ["web/App.tsx", "tsx", "TSX"],
    ["Cargo.lock", "toml", "TOML"],
    ["assets/logo.png", "text", "PNG"],
    ["Dockerfile", "docker", "DOCKER"],
  ])("maps %s to a consistent language and label", (path, language, label) => {
    const visual = workspaceFileVisual(path);
    expect({ language: visual.language, label: visual.label }).toEqual({ language, label });
  });
});
