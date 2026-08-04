import * as fs from "fs";
import * as yaml from "js-yaml";

export const BUILTIN_TOOLS = [
  "home.list",
  "home.read",
  "home.write",
  "home.run",
  "local_tools.search_docs",
  "local_tools.read_file",
] as const;

export interface ProviderHistory {
  max_tail_messages?: number;
  max_chars?: number;
}

export interface ProviderConfig {
  base_url: string;
  model: string;
  api_key_env?: string;
  api_key?: string;
  timeout_ms?: number;
  max_retries?: number;
  retry_base_delay_ms?: number;
  history?: ProviderHistory;
}

export interface McpAuthConfig {
  client_id?: string;
  client_secret_env?: string;
  client_id_metadata_url?: string;
  scopes?: string[];
  redirect_port?: number;
  token_store?: string;
}

export interface McpServerConfig {
  name: string;
  transport?: "stdio" | "http";
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  headers?: Record<string, string>;
  auth?: McpAuthConfig;
  timeout_ms?: number;
}

export interface LimitsConfig {
  max_iterations: number;
  max_tokens: number;
  max_duration_sec: number;
}

export interface LoggingConfig {
  enable_ai_log?: boolean;
  enable_mcp_log?: boolean;
  enable_chat_history?: boolean;
  output_dir?: string;
  sink?: { type: string; path?: string; connection_string?: string };
}

export interface ProgramPolicy {
  name: string;
  executable: string;
  runtime_read_roots?: string[];
  inherit_env?: string[];
  allow_children?: boolean;
}

export interface AccessConfig {
  mode?: string;
  tools?: { builtins?: string[] };
  filesystem?: {
    home?: { read?: string[]; write?: string[] };
    workspace?: { root?: string; read?: string[] };
    input_roots?: string[];
  };
  run?: {
    programs?: ProgramPolicy[];
    max_args?: number;
    max_arg_chars?: number;
    max_output_chars?: number;
    max_timeout_ms?: number;
  };
}

export interface AppConfig {
  provider: ProviderConfig;
  mcp?: McpServerConfig[];
  limits: LimitsConfig;
  logging?: LoggingConfig;
  access?: AccessConfig;
  [key: string]: unknown;
}

/** Editable subset sent to / from the webview. */
export interface EditableAgentConfig {
  provider: {
    base_url: string;
    model: string;
    api_key_env: string;
    timeout_ms: number;
    max_retries: number;
    retry_base_delay_ms: number;
    history: {
      max_tail_messages: number;
      max_chars: number;
    };
  };
  limits: LimitsConfig;
  mcp: McpServerConfig[];
  logging: {
    enable_ai_log: boolean;
    enable_mcp_log: boolean;
    enable_chat_history: boolean;
  };
  access: {
    builtins: string[];
    homeRead: string[];
    homeWrite: string[];
    workspaceRoot: string;
    workspaceRead: string[];
    programs: ProgramPolicy[];
  };
}

export function loadAppConfig(filePath: string): AppConfig {
  const raw = fs.readFileSync(filePath, "utf8");
  const doc = yaml.load(raw);
  if (!doc || typeof doc !== "object") {
    throw new Error("Config is empty or not a mapping");
  }
  return doc as AppConfig;
}

export function toEditable(config: AppConfig): EditableAgentConfig {
  const history = config.provider.history ?? {};
  const access = config.access ?? {};
  const fsPolicy = access.filesystem ?? {};
  const home = fsPolicy.home ?? {};
  const workspace = fsPolicy.workspace ?? {};
  const logging = config.logging ?? {};

  return {
    provider: {
      base_url: config.provider.base_url ?? "",
      model: config.provider.model ?? "",
      api_key_env: config.provider.api_key_env ?? "OPENAI_API_KEY",
      timeout_ms: config.provider.timeout_ms ?? 60_000,
      max_retries: config.provider.max_retries ?? 3,
      retry_base_delay_ms: config.provider.retry_base_delay_ms ?? 500,
      history: {
        max_tail_messages: history.max_tail_messages ?? 30,
        max_chars: history.max_chars ?? 200_000,
      },
    },
    limits: {
      max_iterations: config.limits?.max_iterations ?? 10,
      max_tokens: config.limits?.max_tokens ?? 15_000,
      max_duration_sec: config.limits?.max_duration_sec ?? 120,
    },
    mcp: Array.isArray(config.mcp) ? structuredClone(config.mcp) : [],
    logging: {
      enable_ai_log: logging.enable_ai_log ?? true,
      enable_mcp_log: logging.enable_mcp_log ?? true,
      enable_chat_history: logging.enable_chat_history ?? false,
    },
    access: {
      builtins: [...(access.tools?.builtins ?? [...BUILTIN_TOOLS])],
      homeRead: [...(home.read ?? ["in", "out"])],
      homeWrite: [...(home.write ?? ["out"])],
      workspaceRoot: workspace.root ?? "",
      workspaceRead: [...(workspace.read ?? [])],
      programs: structuredClone(access.run?.programs ?? []),
    },
  };
}

export function applyEditable(base: AppConfig, edit: EditableAgentConfig): AppConfig {
  const next: AppConfig = structuredClone(base);

  next.provider = {
    ...next.provider,
    base_url: edit.provider.base_url,
    model: edit.provider.model,
    api_key_env: edit.provider.api_key_env,
    timeout_ms: edit.provider.timeout_ms,
    max_retries: edit.provider.max_retries,
    retry_base_delay_ms: edit.provider.retry_base_delay_ms,
    history: {
      ...(next.provider.history ?? {}),
      max_tail_messages: edit.provider.history.max_tail_messages,
      max_chars: edit.provider.history.max_chars,
    },
  };
  // Never persist secrets from the form.
  delete next.provider.api_key;

  next.limits = { ...edit.limits };
  next.mcp = normalizeMcp(edit.mcp);

  next.logging = {
    ...(next.logging ?? {}),
    enable_ai_log: edit.logging.enable_ai_log,
    enable_mcp_log: edit.logging.enable_mcp_log,
    enable_chat_history: edit.logging.enable_chat_history,
  };

  const access: AccessConfig = { ...(next.access ?? {}) };
  access.tools = { ...(access.tools ?? {}), builtins: [...edit.access.builtins] };
  access.filesystem = {
    ...(access.filesystem ?? {}),
    home: {
      read: [...edit.access.homeRead],
      write: [...edit.access.homeWrite],
    },
  };
  if (edit.access.workspaceRoot.trim()) {
    access.filesystem.workspace = {
      root: edit.access.workspaceRoot.trim(),
      read: [...edit.access.workspaceRead],
    };
  } else if (access.filesystem.workspace) {
    delete access.filesystem.workspace;
  }

  access.run = {
    ...(access.run ?? {}),
    programs: edit.access.programs.map((p) => ({
      name: p.name,
      executable: p.executable,
      runtime_read_roots: p.runtime_read_roots ?? [],
      inherit_env: p.inherit_env ?? [],
      allow_children: p.allow_children ?? false,
    })),
  };

  next.access = access;
  return next;
}

function normalizeMcp(servers: McpServerConfig[]): McpServerConfig[] {
  return servers.map((s) => {
    const out: McpServerConfig = {
      name: s.name,
      timeout_ms: s.timeout_ms ?? 20_000,
    };
    const isHttp = s.transport === "http" || Boolean(s.url && !s.command);
    if (isHttp) {
      out.transport = "http";
      out.url = s.url ?? "";
      if (s.headers && Object.keys(s.headers).length > 0) {
        out.headers = s.headers;
      }
      if (s.auth) {
        out.auth = s.auth;
      }
    } else {
      out.transport = "stdio";
      out.command = s.command ?? "";
      out.args = s.args ?? [];
      if (s.env && Object.keys(s.env).length > 0) {
        out.env = s.env;
      }
    }
    return out;
  });
}

export function saveAppConfig(filePath: string, config: AppConfig): void {
  const text = yaml.dump(config, {
    lineWidth: 120,
    noRefs: true,
    sortKeys: false,
  });
  fs.writeFileSync(filePath, text, "utf8");
}
