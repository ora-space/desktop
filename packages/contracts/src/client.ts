import {
  endpoints,
  type EndpointOperation,
  type EndpointPathParam,
  type EndpointQueryParam,
  type RequestByOperation,
  type ResponseByOperation,
} from "./endpoints.js";
import type {
  ContractCallOptions,
  ContractTransport,
  ContractTransportRequest,
} from "./transport.js";

type ClientRequestShape = object;

type ClientOperation<Operation extends EndpointOperation> = (
  request: RequestByOperation[Operation],
  options?: ContractCallOptions,
) => (typeof endpoints)[Operation]["responseMode"] extends "stream"
  ? AsyncIterable<ResponseByOperation[Operation]>
  : Promise<ResponseByOperation[Operation]>;

/**
 * Namespaces declared by the endpoint manifest. Sourced from `endpoints` so
 * adding a route in Rust without re-exporting contracts cannot leave the
 * hand-written client silently out of sync.
 */
type EndpointNamespace = (typeof endpoints)[EndpointOperation]["namespace"];

/**
 * Typed shape of the contracts client, derived from the generated `endpoints`
 * manifest. Each endpoint in `ora-contracts` declares a `namespace` and
 * `memberName`; this type re-groups the flat `EndpointOperation` union into
 * the nested shape used at the call site (`client.project.create`).
 *
 * Because the shape is derived from the manifest and `createContractsClient`
 * returns an object literal checked against this type, adding a route in Rust
 * without updating `client.ts` fails `tsc` with a missing-property error,
 * keeping the hand-written client in compile-time lockstep with the routes.
 */
export type ContractsClient = {
  [Namespace in EndpointNamespace]: {
    [Operation in EndpointOperation as (typeof endpoints)[Operation]["namespace"] extends Namespace
      ? (typeof endpoints)[Operation]["memberName"]
      : never]: ClientOperation<Operation>;
  };
};

export function createContractsClient(
  transport: ContractTransport,
): ContractsClient {
  return {
    project: {
      create: (request, options) =>
        executeOperation("createProject", request, transport, options),
      get: (request, options) => executeOperation("getProject", request, transport, options),
      list: (request, options) => executeOperation("listProjects", request, transport, options),
      update: (request, options) =>
        executeOperation("updateProject", request, transport, options),
      delete: (request, options) =>
        executeOperation("deleteProject", request, transport, options),
    },
    projectWorkContext: {
      open: (request, options) =>
        executeOperation("openProjectWorkContext", request, transport, options),
      renew: (request, options) =>
        executeOperation("renewProjectWorkContext", request, transport, options),
    },
    task: {
      create: (request, options) => executeOperation("createTask", request, transport, options),
      get: (request, options) => executeOperation("getTask", request, transport, options),
      list: (request, options) => executeOperation("listTasks", request, transport, options),
      update: (request, options) => executeOperation("updateTask", request, transport, options),
      delete: (request, options) => executeOperation("deleteTask", request, transport, options),
    },
    session: {
      create: (request, options) =>
        executeOperation("createSession", request, transport, options),
      get: (request, options) => executeOperation("getSession", request, transport, options),
      list: (request, options) => executeOperation("listSessions", request, transport, options),
      load: (request, options) => executeStreamOperation("loadSession", request, transport, options),
      prompt: (request, options) => executeStreamOperation("promptSession", request, transport, options),
      respondToPermission: (request, options) => executeOperation("respondToSessionPermission", request, transport, options),
      stop: (request, options) => executeOperation("stopSession", request, transport, options),
      delete: (request, options) =>
        executeOperation("deleteSession", request, transport, options),
    },
    skill: {
      create: (request, options) => executeOperation("createSkill", request, transport, options),
      get: (request, options) => executeOperation("getSkill", request, transport, options),
      list: (request, options) => executeOperation("listSkills", request, transport, options),
      update: (request, options) => executeOperation("updateSkill", request, transport, options),
      delete: (request, options) => executeOperation("deleteSkill", request, transport, options),
    },
    agent: {
      create: (request, options) => executeOperation("createAgent", request, transport, options),
      get: (request, options) => executeOperation("getAgent", request, transport, options),
      list: (request, options) => executeOperation("listAgents", request, transport, options),
      update: (request, options) => executeOperation("updateAgent", request, transport, options),
      delete: (request, options) => executeOperation("deleteAgent", request, transport, options),
    },
    fileSystem: {
      listDirectory: (request, options) =>
        executeOperation("listDirectory", request, transport, options),
    },
    gitIdentity: {
      get: (request, options) =>
        executeOperation("getGitIdentity", request, transport, options),
    },
  };
}

async function executeOperation<Operation extends EndpointOperation>(
  operation: Operation,
  request: RequestByOperation[Operation],
  transport: ContractTransport,
  options?: ContractCallOptions,
): Promise<ResponseByOperation[Operation]> {
  const endpoint = endpoints[operation];
  const path = buildPath(
    endpoint.pathTemplate,
    endpoint.pathParams,
    endpoint.queryParams,
    request as ClientRequestShape,
  );
  const body = buildJsonBody(
    endpoint.pathParams,
    endpoint.queryParams,
    endpoint.hasJsonBody,
    request as ClientRequestShape,
  );
  const transportRequest: ContractTransportRequest = {
    operationName: endpoint.operationName,
    request,
    method: endpoint.method,
    path,
    body,
    headers: buildHeaders(endpoint.hasJsonBody),
  };

  return transport.send<ResponseByOperation[Operation]>(transportRequest, options);
}

/** Builds one typed request and delegates stream lifecycle to the selected transport. */
function executeStreamOperation<Operation extends EndpointOperation>(
  operation: Operation,
  request: RequestByOperation[Operation],
  transport: ContractTransport,
  options?: ContractCallOptions,
): AsyncIterable<ResponseByOperation[Operation]> {
  const endpoint = endpoints[operation];
  const requestShape = request as ClientRequestShape;
  return transport.stream<ResponseByOperation[Operation]>({
    operationName: endpoint.operationName,
    request,
    method: endpoint.method,
    path: buildPath(endpoint.pathTemplate, endpoint.pathParams, endpoint.queryParams, requestShape),
    body: buildJsonBody(endpoint.pathParams, endpoint.queryParams, endpoint.hasJsonBody, requestShape),
    headers: buildHeaders(endpoint.hasJsonBody),
  }, options);
}

function buildPath(
  pathTemplate: string,
  pathParams: readonly EndpointPathParam[],
  queryParams: readonly EndpointQueryParam[],
  request: ClientRequestShape,
): string {
  const requestRecord = request as Record<string, unknown>;
  let path = pathTemplate;

  for (const pathParam of pathParams) {
    const value = requestRecord[pathParam.wireName];

    if (value === undefined || value === null) {
      throw new Error(`missing path parameter ${pathParam.wireName}`);
    }

    path = path.replace(
      `{${pathParam.wireName}}`,
      encodeURIComponent(String(value)),
    );
  }

  const query = new URLSearchParams();

  for (const queryParam of queryParams) {
    const value = requestRecord[queryParam.wireName];

    if (value !== undefined && value !== null) {
      query.append(queryParam.wireName, String(value));
    }
  }

  const queryString = query.toString();
  return queryString === "" ? path : `${path}?${queryString}`;
}

function buildJsonBody(
  pathParams: readonly EndpointPathParam[],
  queryParams: readonly EndpointQueryParam[],
  hasJsonBody: boolean,
  request: ClientRequestShape,
): Record<string, unknown> | undefined {
  if (!hasJsonBody) {
    return undefined;
  }

  const requestRecord = request as Record<string, unknown>;
  const pathParamNames = new Set(
    pathParams.map((pathParam) => pathParam.wireName),
  );

  const queryParamNames = new Set(
    queryParams.map((queryParam) => queryParam.wireName),
  );

  return Object.fromEntries(
    Object.entries(requestRecord).filter(
      ([fieldName]) =>
        !pathParamNames.has(fieldName) && !queryParamNames.has(fieldName),
    ),
  );
}

function buildHeaders(hasJsonBody: boolean): Record<string, string> {
  if (!hasJsonBody) {
    return {};
  }

  return {
    "content-type": "application/json",
  };
}
