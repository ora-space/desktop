/**
 * Splits a byte stream into newline-delimited strings, tolerating CRLF and empty lines.
 *
 * ACP runs over a child process's stdio as one JSON object per line; this codec is the only
 * framing an ACP bridge adds on top of Ora's own binary frames.
 */
export async function* decodeLines(
  readable: ReadableStream<Uint8Array>,
): AsyncGenerator<string> {
  const reader = readable.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) {
        break;
      }
      buffer += decoder.decode(value, { stream: true });
      while (true) {
        const newline = buffer.indexOf("\n");
        if (newline === -1) {
          break;
        }
        const line = buffer.slice(0, newline);
        buffer = buffer.slice(newline + 1);
        if (line.length > 0) {
          yield stripCarriageReturn(line);
        }
      }
    }
    const tail = decoder.decode() + buffer;
    if (tail.length > 0) {
      yield stripCarriageReturn(tail);
    }
  } finally {
    reader.releaseLock();
  }
}

/** Encodes one ACP NDJSON line for a child process's stdin pipe. */
export function encodeLine(line: string): Uint8Array {
  return new TextEncoder().encode(line + "\n");
}

function stripCarriageReturn(line: string): string {
  return line.endsWith("\r") ? line.slice(0, -1) : line;
}
