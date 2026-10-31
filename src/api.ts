import { invoke as tauriInvoke } from "@tauri-apps/api/core";

export interface Host {
  id: string;
  alias: string;
  hostname: string;
  port: number;
  username: string;
  enabled: boolean;
  password_configured: boolean;
  host_key_fingerprint: string | null;
  created_at: string;
  updated_at: string;
}

export interface HostPayload {
  hostId?: string;
  alias: string;
  hostname: string;
  port: number;
  username: string;
  password?: string;
  enabled: boolean;
}

export interface McpAccess {
  endpoint: string;
  token: string;
  running: boolean;
  error: string | null;
}

export interface AppState {
  data_dir: string;
  mcp: McpAccess;
  startup_error: string | null;
}

export interface ProbeResult {
  host_id: string;
  alias: string;
  fingerprint: string;
  host_key_line: string;
}

export interface AutostartState {
  enabled: boolean;
  command: string;
}

async function invoke<T>(name: string, body?: unknown): Promise<T> {
  return body === undefined
    ? tauriInvoke<T>(name)
    : tauriInvoke<T>(name, { body });
}

export const api = {
  listHosts: () => invoke<Host[]>("list_hosts"),
  appState: () => invoke<AppState>("get_app_state"),
  saveHost: (body: HostPayload) => invoke<Host>("save_host", body),
  deleteHost: (hostId: string) => invoke<void>("delete_host", { hostId }),
  probeHost: (host: string) => invoke<ProbeResult>("probe_host", { host }),
  trustHost: (data: ProbeResult) =>
    invoke<Host>("trust_host", {
      hostId: data.host_id,
      fingerprint: data.fingerprint,
      hostKeyLine: data.host_key_line,
    }),
  testHost: (host: string) => invoke<string>("test_host", { host }),
  getAutostart: () => invoke<AutostartState>("get_autostart"),
  setAutostart: (enabled: boolean) =>
    invoke<AutostartState>("set_autostart", { enabled }),
  quit: () => invoke<void>("quit_app"),
};
