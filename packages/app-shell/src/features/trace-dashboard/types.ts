/**
 * Resolved dashboard endpoint returned by the Ora Desktop `get_dashboard_url`
 * command. The command resolves the trace file, writes the locator, probes the
 * externally-managed Streamlit server, and hands the frontend only this payload
 * — the private agent session id never leaves Rust (ADR-0003).
 */
export interface DashboardEndpoint {
  /** Loopback host the dashboard server is configured on (e.g. "127.0.0.1"). */
  host: string;
  /** Configured dashboard port. */
  port: number;
  /** Full iframe URL carrying only the Ora session id and canonical agent type. */
  url: string;
  /** Whether the dashboard server answered the health probe on host:port. */
  serverReachable: boolean;
}

/**
 * Resolves one Ora session id to a dashboard iframe endpoint. Implemented by
 * the Desktop app via a Tauri `get_dashboard_url` invoke; injected so the
 * app-shell stays transport-agnostic and unit-testable without Tauri.
 */
export type DashboardResolver = (sessionId: string) => Promise<DashboardEndpoint>;
