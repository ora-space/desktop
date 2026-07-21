const DEFAULT_CWD = "/workspace/ora";

const APP_PATH = "src/app.ts";
const TEST_PATH = "src/app.test.ts";

const INITIAL_APP_SOURCE = `export function greet(name: string) {
  return \`Hello, \${name}\`;
}
`;

const UPDATED_APP_SOURCE = `export function greet(name: string) {
  const normalizedName = name.trim();
  return \`Hello, \${normalizedName}\`;
}
`;

const INITIAL_TEST_SOURCE = `import { greet } from "./app";

describe("greet", () => {
  it("greets a named user", () => {
    expect(greet("Ora")).toBe("Hello, Ora");
  });
});
`;

const UPDATED_TEST_SOURCE = `import { greet } from "./app";

describe("greet", () => {
  it("greets a named user", () => {
    expect(greet("Ora")).toBe("Hello, Ora");
  });

  it("normalizes surrounding whitespace", () => {
    expect(greet("  Ora  ")).toBe("Hello, Ora");
  });
});
`;

interface VirtualSessionFiles {
  cwd: string;
  files: Map<string, string>;
}

/** Maintains deterministic project fixtures without reading or writing the host filesystem. */
export class MockVirtualFileSystem {
  private readonly sessions = new Map<string, VirtualSessionFiles>();

  /** Creates isolated files for one ACP session. */
  createSession(sessionId: string, cwd: string): void {
    this.sessions.set(sessionId, {
      cwd,
      files: new Map([
        [APP_PATH, INITIAL_APP_SOURCE],
        [TEST_PATH, INITIAL_TEST_SOURCE],
      ]),
    });
  }

  /** Ensures seeded sessions receive the same fixtures as newly created sessions. */
  ensureSession(sessionId: string): void {
    if (!this.sessions.has(sessionId)) this.createSession(sessionId, DEFAULT_CWD);
  }

  /** Reads one relative fixture path from an ACP session. */
  read(sessionId: string, relativePath: string): string | undefined {
    return this.sessions.get(sessionId)?.files.get(relativePath);
  }

  /** Replaces one relative fixture and returns its previous contents. */
  write(sessionId: string, relativePath: string, contents: string): string | undefined {
    const session = this.sessions.get(sessionId);
    if (!session) return undefined;
    const previous = session.files.get(relativePath);
    if (previous === undefined) return undefined;
    session.files.set(relativePath, contents);
    return previous;
  }

  /** Produces an absolute display path for ACP locations and diffs. */
  absolutePath(sessionId: string, relativePath: string): string {
    const cwd = this.sessions.get(sessionId)?.cwd ?? DEFAULT_CWD;
    return `${cwd.replace(/[\\/]+$/, "")}/${relativePath}`;
  }
}

export const mockFileFixtures = {
  appPath: APP_PATH,
  testPath: TEST_PATH,
  updatedAppSource: UPDATED_APP_SOURCE,
  updatedTestSource: UPDATED_TEST_SOURCE,
} as const;
