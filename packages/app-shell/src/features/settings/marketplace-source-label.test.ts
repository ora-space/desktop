import { describe, expect, it } from "vitest";
import { marketplaceSourceLabels } from "./marketplace-source-label";

/** Labels as a plain object so each case compares the whole result at once. */
function labelsOf(urls: string[]): Record<string, string> {
  return Object.fromEntries(marketplaceSourceLabels(urls));
}

describe("marketplaceSourceLabels", () => {
  it("names a source by the last segment of its normalized URL", () => {
    expect(
      labelsOf([
        "https://github.com/acme/awesome-plugins",
        "https://gitlab.com/group/sub/Tools.git/",
        "https://dev.azure.com/org/project/_git/registry",
        "git@bitbucket.org:team/snippets.git",
      ]),
    ).toEqual({
      "https://github.com/acme/awesome-plugins": "awesome-plugins",
      "https://gitlab.com/group/sub/Tools.git/": "tools",
      "https://dev.azure.com/org/project/_git/registry": "registry",
      "git@bitbucket.org:team/snippets.git": "snippets",
    });
  });

  it("tells a fork apart from the repository it shares a name with", () => {
    expect(
      labelsOf([
        "https://github.com/ora-space/marketplace",
        "https://github.com/obsismc/marketplace",
      ]),
    ).toEqual({
      "https://github.com/ora-space/marketplace": "ora-space/marketplace",
      "https://github.com/obsismc/marketplace": "obsismc/marketplace",
    });
  });

  it("elides the shared middle segments and reaches the host when it must", () => {
    expect(
      labelsOf([
        "https://github.com/acme/team/plugins",
        "https://gitlab.com/acme/team/plugins",
        "https://example.com:8443/acme/team/plugins",
      ]),
    ).toEqual({
      "https://github.com/acme/team/plugins": "github.com/…/plugins",
      "https://gitlab.com/acme/team/plugins": "gitlab.com/…/plugins",
      "https://example.com:8443/acme/team/plugins":
        "example.com:8443/…/plugins",
    });
  });

  it("does not treat two spellings of one source as a collision", () => {
    expect(
      labelsOf([
        "https://github.com/acme/plugins",
        "https://github.com/acme/plugins.git",
      ]),
    ).toEqual({
      "https://github.com/acme/plugins": "plugins",
      "https://github.com/acme/plugins.git": "plugins",
    });
  });

  it("falls back to the whole address when no single segment separates the sources", () => {
    expect(
      labelsOf([
        "https://h.example/a/x/plugins",
        "https://h.example/b/x/plugins",
        "https://h.example/a/y/plugins",
      ]),
    ).toEqual({
      "https://h.example/a/x/plugins": "h.example/a/x/plugins",
      "https://h.example/b/x/plugins": "b/…/plugins",
      "https://h.example/a/y/plugins": "y/plugins",
    });
  });
});
