import { endpoints, type EndpointOperation, type EndpointPathParam, type RequestByOperation, type ResponseByOperation } from "./endpoints.js";
import type { ContractTransport, ContractTransportRequest } from "./transport.js";

type ClientRequestShape = object;

type ClientOperation<Operation extends EndpointOperation> = (
  request: RequestByOperation[Operation],
) => Promise<ResponseByOperation[Operation]>;

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

export function createContractsClient(transport: ContractTransport): ContractsClient {
  return {
    project: {
      create: (request) => executeOperation("createProject", request, transport),
      get: (request) => executeOperation("getProject", request, transport),
      list: (request) => executeOperation("listProjects", request, transport),
      update: (request) => executeOperation("updateProject", request, transport),
      delete: (request) => executeOperation("deleteProject", request, transport),
    },
    projectWorkContext: {
      open: (request) => executeOperation("openProjectWorkContext", request, transport),
      renew: (request) => executeOperation("renewProjectWorkContext", request, transport),
    },
    task: {
      create: (request) => executeOperation("createTask", request, transport),
      get: (request) => executeOperation("getTask", request, transport),
      list: (request) => executeOperation("listTasks", request, transport),
      update: (request) => executeOperation("updateTask", request, transport),
      delete: (request) => executeOperation("deleteTask", request, transport),
    },
    session: {
      create: (request) => executeOperation("createSession", request, transport),
      get: (request) => executeOperation("getSession", request, transport),
      list: (request) => executeOperation("listSessions", request, transport),
      update: (request) => executeOperation("updateSession", request, transport),
      delete: (request) => executeOperation("deleteSession", request, transport),
    },
    skill: {
      create: (request) => executeOperation("createSkill", request, transport),
      get: (request) => executeOperation("getSkill", request, transport),
      list: (request) => executeOperation("listSkills", request, transport),
      update: (request) => executeOperation("updateSkill", request, transport),
      delete: (request) => executeOperation("deleteSkill", request, transport),
    },
    agent: {
      create: (request) => executeOperation("createAgent", request, transport),
      get: (request) => executeOperation("getAgent", request, transport),
      list: (request) => executeOperation("listAgents", request, transport),
      update: (request) => executeOperation("updateAgent", request, transport),
      delete: (request) => executeOperation("deleteAgent", request, transport),
    },
  };
}

async function executeOperation<Operation extends EndpointOperation>(
  operation: Operation,
  request: RequestByOperation[Operation],
  transport: ContractTransport,
): Promise<ResponseByOperation[Operation]> {
  const endpoint = endpoints[operation];
  const path = buildPath(endpoint.pathTemplate, endpoint.pathParams, request as ClientRequestShape);
  const body = buildJsonBody(endpoint.pathParams, endpoint.hasJsonBody, request as ClientRequestShape);
  const transportRequest: ContractTransportRequest = {
    operationName: endpoint.operationName,
    method: endpoint.method,
    path,
    body,
    headers: buildHeaders(endpoint.hasJsonBody),
  };

  return transport.send<ResponseByOperation[Operation]>(transportRequest);
}

function buildPath(
  pathTemplate: string,
  pathParams: readonly EndpointPathParam[],
  request: ClientRequestShape,
): string {
  const requestRecord = request as Record<string, unknown>;
  let path = pathTemplate;

  for (const pathParam of pathParams) {
    const value = requestRecord[pathParam.wireName];

    if (value === undefined || value === null) {
      throw new Error(`missing path parameter ${pathParam.wireName}`);
    }

    path = path.replace(`{${pathParam.wireName}}`, encodeURIComponent(String(value)));
  }

  return path;
}

function buildJsonBody(
  pathParams: readonly EndpointPathParam[],
  hasJsonBody: boolean,
  request: ClientRequestShape,
): Record<string, unknown> | undefined {
  if (!hasJsonBody) {
    return undefined;
  }

  const requestRecord = request as Record<string, unknown>;
  const pathParamNames = new Set(pathParams.map((pathParam) => pathParam.wireName));

  return Object.fromEntries(
    Object.entries(requestRecord).filter(([fieldName]) => !pathParamNames.has(fieldName)),
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
