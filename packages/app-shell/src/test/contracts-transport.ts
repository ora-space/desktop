import {
  createContractsClient,
  endpoints,
  type ContractCallOptions,
  type ContractTransport,
  type ContractTransportRequest,
  type EndpointOperation,
  type RequestByOperation,
  type ResponseByOperation,
} from "@ora/contracts";

/** A test implements operations, never a second hand-maintained ContractsClient shape. */
export type OperationHandler<Operation extends EndpointOperation> = (
  request: RequestByOperation[Operation],
  options?: ContractCallOptions,
) => (typeof endpoints)[Operation]["responseMode"] extends "stream"
  ? AsyncIterable<ResponseByOperation[Operation]>
  : ResponseByOperation[Operation] | Promise<ResponseByOperation[Operation]>;

/** Tests explicitly compose the operations they need; every omitted operation fails. */
export type TestHandlers = {
  [Operation in EndpointOperation]?: OperationHandler<Operation>;
};

type ErasedHandler = (
  request: unknown,
  options?: ContractCallOptions,
) => unknown;

/** Routes the real generated client to typed in-memory operations, with no success defaults. */
export function createTestTransport(handlers: TestHandlers): ContractTransport {
  const resolve = (
    operationName: string,
    mode: "unary" | "stream",
  ): ErasedHandler => {
    if (!Object.hasOwn(endpoints, operationName)) {
      throw new Error(`Unknown test operation: ${operationName}`);
    }
    const operation = operationName as EndpointOperation;
    if (endpoints[operation].responseMode !== mode) {
      throw new Error(
        `Wrong test transport mode for ${operationName}: ${mode}`,
      );
    }
    if (
      !Object.hasOwn(handlers, operation) ||
      handlers[operation] === undefined
    ) {
      throw new Error(`Unconfigured test operation: ${operationName}`);
    }
    // Registration preserves request/result correlation through TestHandlers. The transport
    // interface erases that correlation to an operation name and unknown DTO, just as IPC does.
    return handlers[operation] as unknown as ErasedHandler;
  };

  return {
    async send<TResponse>(
      { operationName, request }: ContractTransportRequest,
      options?: ContractCallOptions,
    ) {
      return (await resolve(operationName, "unary")(
        request,
        options,
      )) as TResponse;
    },
    stream<TEvent>(
      { operationName, request }: ContractTransportRequest,
      options?: ContractCallOptions,
    ) {
      return (async function* () {
        // Resolve on consumption so opening a cold stream performs no operation or failure.
        const events = resolve(operationName, "stream")(
          request,
          options,
        ) as AsyncIterable<TEvent>;
        yield* events;
      })();
    },
  };
}

/** Uses production request construction and namespace wiring with only the transport substituted. */
export function createTestClient(handlers: TestHandlers) {
  return createContractsClient(createTestTransport(handlers));
}
