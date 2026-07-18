export type HttpMethod = "GET" | "POST" | "PUT" | "DELETE";

export type ContractTransportRequest = {
  operationName: string;
  method: HttpMethod;
  path: string;
  body: unknown | undefined;
  headers: Record<string, string>;
};

export interface ContractTransport {
  send<TResponse>(request: ContractTransportRequest): Promise<TResponse>;
}

export type ContractErrorPayload = {
  code: string;
  message: string;
};

export type ContractErrorEnvelope = {
  error: ContractErrorPayload;
};

export class ContractTransportError extends Error {
  readonly code: string;
  readonly status: number | null;
  readonly responseBody: unknown;

  constructor({
    code,
    message,
    status,
    responseBody,
  }: {
    code: string;
    message: string;
    status: number | null;
    responseBody: unknown;
  }) {
    super(message);
    this.name = "ContractTransportError";
    this.code = code;
    this.status = status;
    this.responseBody = responseBody;
  }
}
