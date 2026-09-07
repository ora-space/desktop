import {
  endpoints,
  type EndpointOperation,
  type RequestByOperation,
  type ResponseByOperation,
} from "./endpoints.ts";
import type {
  ContractCallOptions,
  ContractTransport,
  ContractTransportRequest,
} from "./transport.ts";

type ClientOperation<Operation extends EndpointOperation> = (
  request: RequestByOperation[Operation],
  options?: ContractCallOptions,
) => (typeof endpoints)[Operation]["responseMode"] extends "stream"
  ? AsyncIterable<ResponseByOperation[Operation]>
  : Promise<ResponseByOperation[Operation]>;

/** Namespaces owned by the generated operation manifest. */
type EndpointNamespace = (typeof endpoints)[EndpointOperation]["namespace"];

/** The generated factory is checked against this operation-derived client interface. */
export type ContractsClient = {
  [Namespace in EndpointNamespace]: {
    [
      Operation in EndpointOperation as (typeof endpoints)[Operation]["namespace"] extends Namespace
        ? (typeof endpoints)[Operation]["memberName"]
        : never
    ]: ClientOperation<Operation>;
  };
};

/** Sends the complete DTO and options without transport-specific serialization. */
export async function executeOperation<Operation extends EndpointOperation>(
  operation: Operation,
  request: RequestByOperation[Operation],
  transport: ContractTransport,
  options?: ContractCallOptions,
): Promise<ResponseByOperation[Operation]> {
  const endpoint = endpoints[operation];
  const transportRequest: ContractTransportRequest = {
    operationName: endpoint.operationName,
    request,
  };

  return transport.send<ResponseByOperation[Operation]>(
    transportRequest,
    options,
  );
}

/** Builds one typed request and delegates stream lifecycle to the selected transport. */
export function executeStreamOperation<Operation extends EndpointOperation>(
  operation: Operation,
  request: RequestByOperation[Operation],
  transport: ContractTransport,
  options?: ContractCallOptions,
): AsyncIterable<ResponseByOperation[Operation]> {
  const endpoint = endpoints[operation];
  return transport.stream<ResponseByOperation[Operation]>(
    {
      operationName: endpoint.operationName,
      request,
    },
    options,
  );
}
