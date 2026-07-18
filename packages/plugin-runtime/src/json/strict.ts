const NUMBER_PATTERN = /-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/y;

/** Parses JSON while rejecting duplicate object keys, excessive depth, and non-finite numbers. */
export function parseStrictJson(source: Uint8Array | string, maximumDepth = 64): unknown {
  if (!Number.isInteger(maximumDepth) || maximumDepth < 1) {
    throw new RangeError("maximum JSON depth must be a positive integer");
  }
  const text = typeof source === "string" ? source : new TextDecoder("utf-8", { fatal: true }).decode(source);
  return new StrictJsonParser(text, maximumDepth).parse();
}

class StrictJsonParser {
  #offset = 0;

  constructor(
    readonly source: string,
    readonly maximumDepth: number,
  ) {}

  parse(): unknown {
    this.#skipWhitespace();
    const value = this.#parseValue(1);
    this.#skipWhitespace();
    if (this.#offset !== this.source.length) {
      throw new SyntaxError("trailing content after JSON value");
    }
    return value;
  }

  #parseValue(depth: number): unknown {
    if (depth > this.maximumDepth) {
      throw new SyntaxError("JSON nesting depth exceeded");
    }
    switch (this.source[this.#offset]) {
      case "{":
        return this.#parseObject(depth);
      case "[":
        return this.#parseArray(depth);
      case '"':
        return this.#parseString();
      case "t":
        return this.#parseKeyword("true", true);
      case "f":
        return this.#parseKeyword("false", false);
      case "n":
        return this.#parseKeyword("null", null);
      default:
        return this.#parseNumber();
    }
  }

  #parseObject(depth: number): Record<string, unknown> {
    this.#offset += 1;
    this.#skipWhitespace();
    const result: Record<string, unknown> = {};
    const keys = new Set<string>();
    if (this.source[this.#offset] === "}") {
      this.#offset += 1;
      return result;
    }
    for (;;) {
      if (this.source[this.#offset] !== '"') {
        throw new SyntaxError("JSON object key must be a string");
      }
      const key = this.#parseString();
      if (keys.has(key)) {
        throw new SyntaxError("duplicate JSON object key");
      }
      keys.add(key);
      this.#skipWhitespace();
      this.#consume(":");
      this.#skipWhitespace();
      Object.defineProperty(result, key, {
        value: this.#parseValue(depth + 1),
        enumerable: true,
        configurable: true,
        writable: true,
      });
      this.#skipWhitespace();
      if (this.source[this.#offset] === "}") {
        this.#offset += 1;
        return result;
      }
      this.#consume(",");
      this.#skipWhitespace();
    }
  }

  #parseArray(depth: number): unknown[] {
    this.#offset += 1;
    this.#skipWhitespace();
    const result: unknown[] = [];
    if (this.source[this.#offset] === "]") {
      this.#offset += 1;
      return result;
    }
    for (;;) {
      result.push(this.#parseValue(depth + 1));
      this.#skipWhitespace();
      if (this.source[this.#offset] === "]") {
        this.#offset += 1;
        return result;
      }
      this.#consume(",");
      this.#skipWhitespace();
    }
  }

  #parseString(): string {
    const start = this.#offset;
    this.#offset += 1;
    let escaped = false;
    while (this.#offset < this.source.length) {
      const code = this.source.charCodeAt(this.#offset);
      if (code < 0x20) {
        throw new SyntaxError("unescaped control character in JSON string");
      }
      const character = this.source[this.#offset];
      this.#offset += 1;
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (character === '"') {
        return JSON.parse(this.source.slice(start, this.#offset)) as string;
      }
    }
    throw new SyntaxError("unterminated JSON string");
  }

  #parseNumber(): number {
    NUMBER_PATTERN.lastIndex = this.#offset;
    const match = NUMBER_PATTERN.exec(this.source);
    if (match === null) {
      throw new SyntaxError("invalid JSON value");
    }
    this.#offset = NUMBER_PATTERN.lastIndex;
    const value = Number(match[0]);
    if (!Number.isFinite(value)) {
      throw new SyntaxError("JSON number must be finite");
    }
    return value;
  }

  #parseKeyword<T>(keyword: string, value: T): T {
    if (!this.source.startsWith(keyword, this.#offset)) {
      throw new SyntaxError("invalid JSON keyword");
    }
    this.#offset += keyword.length;
    return value;
  }

  #consume(expected: string): void {
    if (this.source[this.#offset] !== expected) {
      throw new SyntaxError(`expected '${expected}' in JSON`);
    }
    this.#offset += 1;
  }

  #skipWhitespace(): void {
    while (/\s/u.test(this.source[this.#offset] ?? "") && this.#offset < this.source.length) {
      const character = this.source[this.#offset];
      if (character !== " " && character !== "\n" && character !== "\r" && character !== "\t") {
        throw new SyntaxError("invalid JSON whitespace");
      }
      this.#offset += 1;
    }
  }
}
