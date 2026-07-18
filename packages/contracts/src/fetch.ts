import { ContractTransportError, type ContractErrorPayload, type ContractTransport, type ContractTransportRequest } from "./transport.js";

export type FetchTransportOptions = {
  baseUrl?: string;
  fetch?: typeof globalThis.fetch;
};

export function createFetchTransport(
  options: FetchTransportOptions = {},
): ContractTransport {
  const fetchImplementation = options.fetch ?? globalThis.fetch;

  if (fetchImplementation === undefined) {
    throw new Error("global fetch is not available");
  }

  return {
    async send<TResponse>(request: ContractTransportRequest): Promise<TResponse> {
      const response = await fetchImplementation(resolveUrl(options.baseUrl ?? "", request.path), {
        method: request.method,
        headers: request.headers,
        body: request.body === undefined ? undefined : JSON.stringify(request.body),
      });
      const responseBody = await readResponseBody(response);

      if (!response.ok) {
        throw toTransportError(response.status, responseBody);
      }

      return responseBody as TResponse;
    },
  };
}

export function resolveUrl(baseUrl: string, path: string): string {
  if (baseUrl === "") {
    return path;
  }

  return new URL(path, baseUrl).toString();
}

export function decodeErrorEnvelope(body: unknown): ContractErrorPayload | null {
  if (!isRecord(body)) {
    return null;
  }

  const error = body.error;

  if (!isRecord(error) || typeof error.code !== "string" || typeof error.message !== "string") {
    return null;
  }

  return {
    code: error.code,
    message: error.message,
  };
}

async function readResponseBody(response: Response): Promise<unknown> {
  const bodyText = await response.text();

  if (bodyText === "") {
    return null;
  }

  try {
    return JSON.parse(bodyText) as unknown;
  } catch {
    return bodyText;
  }
}

function toTransportError(status: number, responseBody: unknown): ContractTransportError {
  const decodedError = decodeErrorEnvelope(responseBody);

  if (decodedError !== null) {
    return new ContractTransportError({
      code: decodedError.code,
      message: decodedError.message,
      status,
      responseBody,
    });
  }

  return new ContractTransportError({
    code: "http_error",
    message: `HTTP request failed with status ${status}`,
    status,
    responseBody,
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
