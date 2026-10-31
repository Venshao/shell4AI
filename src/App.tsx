import { useEffect, useMemo, useRef, useState } from "react";
import {
  CheckCircle,
  Copy,
  MagnifyingGlass,
  Moon,
  PencilSimple,
  PlugsConnected,
  Plus,
  ShieldCheck,
  Sun,
  Trash,
  WarningCircle,
  X,
} from "@phosphor-icons/react";
import { api, type AppState, type Host, type HostPayload, type ProbeResult } from "./api";

type FormState = {
  alias: string;
  hostname: string;
  port: string;
  username: string;
  password: string;
  enabled: boolean;
};

type ModalKind = "host" | "mcp" | null;
type McpClientTab = "cursor" | "claude" | "codex";
type ThemeMode = "light" | "dark";

const THEME_KEY = "ssh-mcp.theme";

function readStoredTheme(): ThemeMode {
  const stored = window.localStorage.getItem(THEME_KEY);
  if (stored === "light" || stored === "dark") return stored;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function applyTheme(theme: ThemeMode) {
  document.documentElement.setAttribute("data-theme", theme);
}

const EMPTY_FORM: FormState = {
  alias: "",
  hostname: "",
  port: "22",
  username: "root",
  password: "",
  enabled: true,
};

const MCP_TABS: { id: McpClientTab; label: string }[] = [
  { id: "cursor", label: "Cursor" },
  { id: "claude", label: "Claude Code" },
  { id: "codex", label: "Codex" },
];

function buildCursorConfig(endpoint: string, token: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        "ssh-mcp": {
          url: endpoint,
          headers: {
            Authorization: `Bearer ${token}`,
          },
        },
      },
    },
    null,
    2,
  );
}

function buildClaudeConfig(endpoint: string, token: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        "ssh-mcp": {
          type: "http",
          url: endpoint,
          headers: {
            Authorization: `Bearer ${token}`,
          },
        },
      },
    },
    null,
    2,
  );
}

function buildClaudeCli(endpoint: string, token: string): string {
  return `claude mcp add --transport http ssh-mcp ${endpoint} --header "Authorization: Bearer ${token}"`;
}

function buildCodexToml(endpoint: string, token: string): string {
  return `[mcp_servers.ssh-mcp]
url = "${endpoint}"
http_headers = { Authorization = "Bearer ${token}" }`;
}

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return JSON.stringify(error);
}

function shortFingerprint(value: string | null): string {
  if (!value) return "未验证";
  return value.length > 36 ? `${value.slice(0, 24)}…${value.slice(-8)}` : value;
}

function ConfigSnippet({
  title,
  value,
  disabled,
  onCopy,
}: {
  title: string;
  value: string;
  disabled?: boolean;
  onCopy: () => void | Promise<void>;
}) {
  return (
    <div className="config-snippet">
      <div className="secret-label">
        <span>{title}</span>
        <button type="button" className="btn ghost sm" disabled={disabled} onClick={() => void onCopy()}>
          <Copy size={14} aria-hidden="true" />
          复制
        </button>
      </div>
      <pre>{value}</pre>
    </div>
  );
}

export default function App() {
  const [hosts, setHosts] = useState<Host[]>([]);
  const [appState, setAppState] = useState<AppState | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [editing, setEditing] = useState<Host | null>(null);
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const [modal, setModal] = useState<ModalKind>(null);
  const [query, setQuery] = useState("");
  const [autostart, setAutostart] = useState(false);
  const [mcpTab, setMcpTab] = useState<McpClientTab>("cursor");
  const [theme, setTheme] = useState<ThemeMode>(() => {
    const initial = readStoredTheme();
    applyTheme(initial);
    return initial;
  });
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<{ kind: "ok" | "error"; text: string } | null>(null);
  const firstFieldRef = useRef<HTMLInputElement>(null);

  const trustedCount = useMemo(
    () => hosts.filter((host) => host.host_key_fingerprint).length,
    [hosts],
  );

  const filteredHosts = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return hosts;
    return hosts.filter((host) =>
      [host.alias, host.hostname, host.username, String(host.port)]
        .join(" ")
        .toLowerCase()
        .includes(q),
    );
  }, [hosts, query]);

  async function loadAll() {
    try {
      const [nextHosts, state, currentAutostart] = await Promise.all([
        api.listHosts(),
        api.appState(),
        api.getAutostart(),
      ]);
      const preferenceKey = "ssh-mcp.autostart.enabled";
      const storedPreference = window.localStorage.getItem(preferenceKey);
      const desiredEnabled = storedPreference === null ? true : storedPreference === "1";
      let enabled = currentAutostart.enabled;
      if (desiredEnabled !== currentAutostart.enabled) {
        enabled = (await api.setAutostart(desiredEnabled)).enabled;
      }
      window.localStorage.setItem(preferenceKey, enabled ? "1" : "0");
      setHosts(nextHosts);
      setAppState(state);
      setAutostart(enabled);
    } catch (error) {
      setMessage({ kind: "error", text: errorText(error) });
    }
  }

  useEffect(() => {
    void loadAll();
    const timer = window.setInterval(() => {
      void api.appState().then(setAppState).catch(() => undefined);
    }, 5000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    applyTheme(theme);
    window.localStorage.setItem(THEME_KEY, theme);
  }, [theme]);

  function toggleTheme() {
    setTheme((current) => (current === "light" ? "dark" : "light"));
  }

  useEffect(() => {
    if (modal !== "host") return;
    const id = window.setTimeout(() => firstFieldRef.current?.focus(), 30);
    return () => window.clearTimeout(id);
  }, [modal, editing]);

  useEffect(() => {
    if (!modal && !probe) return;
    function onKey(event: KeyboardEvent) {
      if (event.key !== "Escape" || busy) return;
      if (probe) setProbe(null);
      else setModal(null);
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [modal, probe, busy]);

  function openCreate() {
    setEditing(null);
    setForm(EMPTY_FORM);
    setModal("host");
  }

  function openEdit(host: Host) {
    setEditing(host);
    setForm({
      alias: host.alias,
      hostname: host.hostname,
      port: String(host.port),
      username: host.username,
      password: "",
      enabled: host.enabled,
    });
    setModal("host");
  }

  function closeHostModal() {
    if (busy === "save") return;
    setModal(null);
    setEditing(null);
    setForm(EMPTY_FORM);
  }

  async function saveHost(event: React.FormEvent) {
    event.preventDefault();
    setBusy("save");
    setMessage(null);
    try {
      const payload: HostPayload = {
        hostId: editing?.id,
        alias: form.alias.trim(),
        hostname: form.hostname.trim(),
        username: form.username.trim(),
        port: Number(form.port),
        enabled: form.enabled,
      };
      if (form.password) payload.password = form.password;
      await api.saveHost(payload);
      await loadAll();
      setModal(null);
      setEditing(null);
      setForm(EMPTY_FORM);
      setMessage({
        kind: "ok",
        text: editing ? "主机已更新" : "主机已添加，请验证 SSH 指纹",
      });
    } catch (error) {
      setMessage({ kind: "error", text: errorText(error) });
    } finally {
      setBusy(null);
    }
  }

  async function deleteHost(host: Host) {
    if (!window.confirm(`确定删除主机 ${host.alias}？该主机的加密密码也会删除。`)) return;
    setBusy(`delete:${host.id}`);
    try {
      await api.deleteHost(host.id);
      await loadAll();
      if (editing?.id === host.id) closeHostModal();
      setMessage({ kind: "ok", text: "主机已删除" });
    } catch (error) {
      setMessage({ kind: "error", text: errorText(error) });
    } finally {
      setBusy(null);
    }
  }

  async function probeHost(host: Host) {
    setBusy(`probe:${host.id}`);
    setMessage(null);
    try {
      setProbe(await api.probeHost(host.alias));
    } catch (error) {
      setMessage({ kind: "error", text: errorText(error) });
    } finally {
      setBusy(null);
    }
  }

  async function trustProbe() {
    if (!probe) return;
    setBusy("trust");
    try {
      await api.trustHost(probe);
      setProbe(null);
      await loadAll();
      setMessage({ kind: "ok", text: "SSH 主机指纹已保存" });
    } catch (error) {
      setMessage({ kind: "error", text: errorText(error) });
    } finally {
      setBusy(null);
    }
  }

  async function testHost(host: Host) {
    setBusy(`test:${host.id}`);
    setMessage(null);
    try {
      const result = await api.testHost(host.alias);
      setMessage({ kind: "ok", text: `${host.alias}: ${result}` });
    } catch (error) {
      setMessage({ kind: "error", text: errorText(error) });
    } finally {
      setBusy(null);
    }
  }

  async function toggleAutostart() {
    setBusy("autostart");
    try {
      const result = await api.setAutostart(!autostart);
      window.localStorage.setItem("ssh-mcp.autostart.enabled", result.enabled ? "1" : "0");
      setAutostart(result.enabled);
    } catch (error) {
      setMessage({ kind: "error", text: errorText(error) });
    } finally {
      setBusy(null);
    }
  }

  async function copyText(label: string, value: string) {
    await navigator.clipboard.writeText(value);
    setMessage({ kind: "ok", text: `${label} 已复制` });
  }

  const mcpRunning = Boolean(appState?.mcp.running);
  const endpoint = appState?.mcp.endpoint ?? "";
  const token = appState?.mcp.token ?? "";

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand">
          <h1>SSH MCP</h1>
          <p>本机 SSH 主机管理</p>
        </div>

        <div className="topbar-actions">
          <span
            className={`status-text ${mcpRunning ? "online" : "offline"}`}
            title={appState?.mcp.error || undefined}
          >
            {mcpRunning ? "MCP 已启动" : "MCP 未启动"}
          </span>

          <label className="switch-label">
            <input
              type="checkbox"
              checked={autostart}
              onChange={toggleAutostart}
              disabled={busy === "autostart"}
            />
            <span className="switch" />
            登录自启动
          </label>

          <button
            type="button"
            className="icon-btn"
            onClick={toggleTheme}
            aria-label={theme === "light" ? "切换到暗色主题" : "切换到亮色主题"}
            title={theme === "light" ? "暗色" : "亮色"}
          >
            {theme === "light" ? (
              <Moon size={16} aria-hidden="true" />
            ) : (
              <Sun size={16} aria-hidden="true" />
            )}
          </button>

          <button type="button" className="btn ghost" onClick={() => setModal("mcp")}>
            <PlugsConnected size={15} aria-hidden="true" />
            MCP
          </button>
          <button type="button" className="btn primary" onClick={openCreate}>
            <Plus size={15} weight="bold" aria-hidden="true" />
            新增主机
          </button>
        </div>
      </header>

      {(message || appState?.startup_error || appState?.mcp.error) && (
        <div
          className={`banner ${
            message?.kind === "error" || appState?.startup_error || appState?.mcp.error
              ? "error"
              : "success"
          }`}
          role="status"
        >
          <span>{message?.text || appState?.startup_error || appState?.mcp.error}</span>
          {message && (
            <button type="button" className="icon-btn" aria-label="关闭提示" onClick={() => setMessage(null)}>
              <X size={16} aria-hidden="true" />
            </button>
          )}
        </div>
      )}

      <main className="content">
        <section className="list-panel">
          <div className="list-toolbar">
            <div>
              <h2>主机列表</h2>
              <p>
                共 {hosts.length} 台 · 已验证 {trustedCount} 台
              </p>
            </div>
            <label className="search-field">
              <MagnifyingGlass size={16} aria-hidden="true" />
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="搜索别名、IP、用户名…"
                aria-label="搜索主机"
              />
            </label>
          </div>

          {hosts.length === 0 ? (
            <div className="empty-state">
              <h3>还没有主机</h3>
              <p>先添加一台服务器。密码只保存在本机；Agent 使用别名或 IP 即可连接。</p>
              <button type="button" className="btn primary" onClick={openCreate}>
                <Plus size={15} weight="bold" aria-hidden="true" />
                新增主机
              </button>
            </div>
          ) : filteredHosts.length === 0 ? (
            <div className="empty-state compact">
              <h3>没有匹配的主机</h3>
              <p>试试别的关键词，或清空搜索。</p>
            </div>
          ) : (
            <div className="host-table" role="table" aria-label="主机列表">
              <div className="host-table-head" role="row">
                <span role="columnheader">状态</span>
                <span role="columnheader">别名</span>
                <span role="columnheader">连接</span>
                <span role="columnheader">指纹</span>
                <span role="columnheader">操作</span>
              </div>
              {filteredHosts.map((host) => {
                const trusted = Boolean(host.host_key_fingerprint);
                return (
                  <article className="host-row" role="row" key={host.id}>
                    <div className="status-cell" role="cell">
                      <span
                        className={`dot ${trusted ? "trusted" : "pending"} ${!host.enabled ? "disabled" : ""}`}
                        title={trusted ? "已验证" : "待验证"}
                      />
                    </div>
                    <div className="alias-cell" role="cell">
                      <strong>{host.alias}</strong>
                      <div className="tags">
                        {!host.enabled && <span className="tag muted">已禁用</span>}
                        {!trusted && <span className="tag warning">待验证</span>}
                        {trusted && <span className="tag ok">已信任</span>}
                      </div>
                    </div>
                    <div className="mono-cell" role="cell">
                      {host.username}@{host.hostname}:{host.port}
                    </div>
                    <div className="mono-cell muted" role="cell">
                      {shortFingerprint(host.host_key_fingerprint)}
                    </div>
                    <div className="actions-cell" role="cell">
                      <button type="button" className="btn ghost sm" onClick={() => openEdit(host)}>
                        <PencilSimple size={14} aria-hidden="true" />
                        编辑
                      </button>
                      <button
                        type="button"
                        className="btn ghost sm"
                        onClick={() => probeHost(host)}
                        disabled={busy === `probe:${host.id}`}
                      >
                        <ShieldCheck size={14} aria-hidden="true" />
                        {trusted ? "复核" : "验证"}
                      </button>
                      <button
                        type="button"
                        className="btn ghost sm"
                        onClick={() => testHost(host)}
                        disabled={!trusted || !host.password_configured || busy === `test:${host.id}`}
                      >
                        <CheckCircle size={14} aria-hidden="true" />
                        测试
                      </button>
                      <button
                        type="button"
                        className="btn danger sm"
                        onClick={() => deleteHost(host)}
                        disabled={busy === `delete:${host.id}`}
                      >
                        <Trash size={14} aria-hidden="true" />
                        删除
                      </button>
                    </div>
                  </article>
                );
              })}
            </div>
          )}
        </section>
      </main>

      {modal === "host" && (
        <div
          className="modal-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) closeHostModal();
          }}
        >
          <div className="modal" role="dialog" aria-modal="true" aria-labelledby="host-modal-title">
            <div className="modal-header">
              <h2 id="host-modal-title">{editing ? "编辑主机" : "新增主机"}</h2>
              <button type="button" className="icon-btn" aria-label="关闭" onClick={closeHostModal}>
                <X size={16} aria-hidden="true" />
              </button>
            </div>
            <form className="modal-form" onSubmit={saveHost}>
              <label>
                别名
                <input
                  ref={firstFieldRef}
                  value={form.alias}
                  onChange={(event) => setForm({ ...form, alias: event.target.value })}
                  placeholder="prod-web-1"
                  required
                />
              </label>
              <label>
                主机名或 IP
                <input
                  value={form.hostname}
                  onChange={(event) => setForm({ ...form, hostname: event.target.value })}
                  placeholder="10.0.0.10"
                  required
                />
              </label>
              <div className="field-row">
                <label>
                  端口
                  <input
                    type="number"
                    min={1}
                    max={65535}
                    value={form.port}
                    onChange={(event) => setForm({ ...form, port: event.target.value })}
                    required
                  />
                </label>
                <label>
                  用户名
                  <input
                    value={form.username}
                    onChange={(event) => setForm({ ...form, username: event.target.value })}
                    required
                  />
                </label>
              </div>
              <label>
                SSH 密码
                <input
                  type="password"
                  value={form.password}
                  onChange={(event) => setForm({ ...form, password: event.target.value })}
                  placeholder={editing ? "留空表示保持原密码" : "仅保存在本机加密存储"}
                  required={!editing}
                  autoComplete="new-password"
                />
              </label>
              <label className="check-row">
                <input
                  type="checkbox"
                  checked={form.enabled}
                  onChange={(event) => setForm({ ...form, enabled: event.target.checked })}
                />
                启用该主机
              </label>
              <div className="modal-actions">
                <button type="button" className="btn ghost" onClick={closeHostModal}>
                  取消
                </button>
                <button type="submit" className="btn primary" disabled={busy === "save"}>
                  {busy === "save" ? "保存中…" : editing ? "保存修改" : "添加主机"}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {modal === "mcp" && (
        <div
          className="modal-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setModal(null);
          }}
        >
          <div className="modal mcp-modal" role="dialog" aria-modal="true" aria-labelledby="mcp-modal-title">
            <div className="modal-header">
              <h2 id="mcp-modal-title">MCP 接入</h2>
              <button type="button" className="icon-btn" aria-label="关闭" onClick={() => setModal(null)}>
                <X size={16} aria-hidden="true" />
              </button>
            </div>

            <div className="mcp-body">
              <div className={`mcp-status ${mcpRunning ? "online" : "offline"}`}>
                {mcpRunning ? (
                  <CheckCircle size={18} weight="fill" aria-hidden="true" />
                ) : (
                  <WarningCircle size={18} weight="fill" aria-hidden="true" />
                )}
                <div>
                  <strong>{mcpRunning ? "服务可用" : "服务未就绪"}</strong>
                  <p>先保持本应用运行，再把下方配置粘贴到对应 Agent。</p>
                </div>
              </div>

              <div className="mcp-tabs" role="tablist" aria-label="Agent 客户端">
                {MCP_TABS.map((tab) => (
                  <button
                    key={tab.id}
                    type="button"
                    role="tab"
                    aria-selected={mcpTab === tab.id}
                    className={`mcp-tab ${mcpTab === tab.id ? "active" : ""}`}
                    onClick={() => setMcpTab(tab.id)}
                  >
                    {tab.label}
                  </button>
                ))}
              </div>

              {mcpTab === "cursor" && (
                <div className="mcp-panel" role="tabpanel">
                  <ol className="mcp-steps">
                    <li>
                      打开 <code>%USERPROFILE%\.cursor\mcp.json</code>（全局）或项目下
                      {" "}
                      <code>.cursor/mcp.json</code>
                    </li>
                    <li>把下面整段合并进已有 <code>mcpServers</code>（或直接粘贴为完整文件）</li>
                    <li>保存后重启 Cursor，在 Settings → MCP 启用 <code>ssh-mcp</code></li>
                  </ol>
                  <ConfigSnippet
                    title="Cursor mcp.json"
                    value={
                      endpoint && token
                        ? buildCursorConfig(endpoint, token)
                        : "等待 MCP 就绪…"
                    }
                    disabled={!endpoint || !token}
                    onCopy={async () => {
                      if (!endpoint || !token) return;
                      await copyText("Cursor 配置", buildCursorConfig(endpoint, token));
                    }}
                  />
                </div>
              )}

              {mcpTab === "claude" && (
                <div className="mcp-panel" role="tabpanel">
                  <ol className="mcp-steps">
                    <li>推荐：在终端执行下方 CLI（会写入 Claude Code 的 MCP 配置）</li>
                    <li>
                      或手动写入项目 <code>.mcp.json</code> / 用户配置（须带
                      {" "}
                      <code>"type": "http"</code>
                      ）
                    </li>
                    <li>新开 Claude Code 会话，用 <code>/mcp</code> 确认已连接</li>
                  </ol>
                  <ConfigSnippet
                    title="推荐：CLI 一键添加"
                    value={
                      endpoint && token ? buildClaudeCli(endpoint, token) : "等待 MCP 就绪…"
                    }
                    disabled={!endpoint || !token}
                    onCopy={async () => {
                      if (!endpoint || !token) return;
                      await copyText("Claude Code CLI", buildClaudeCli(endpoint, token));
                    }}
                  />
                  <ConfigSnippet
                    title="可选：.mcp.json"
                    value={
                      endpoint && token
                        ? buildClaudeConfig(endpoint, token)
                        : "等待 MCP 就绪…"
                    }
                    disabled={!endpoint || !token}
                    onCopy={async () => {
                      if (!endpoint || !token) return;
                      await copyText("Claude Code 配置", buildClaudeConfig(endpoint, token));
                    }}
                  />
                </div>
              )}

              {mcpTab === "codex" && (
                <div className="mcp-panel" role="tabpanel">
                  <ol className="mcp-steps">
                    <li>
                      打开 <code>%USERPROFILE%\.codex\config.toml</code>
                    </li>
                    <li>把下面整段追加进去（本机 MCP，Token 直接写在配置里即可）</li>
                    <li>保存后重启 Codex</li>
                  </ol>
                  <ConfigSnippet
                    title="Codex config.toml"
                    value={
                      endpoint && token
                        ? buildCodexToml(endpoint, token)
                        : "等待 MCP 就绪…"
                    }
                    disabled={!endpoint || !token}
                    onCopy={async () => {
                      if (!endpoint || !token) return;
                      await copyText("Codex 配置", buildCodexToml(endpoint, token));
                    }}
                  />
                </div>
              )}

              <p className="path-hint">
                数据目录：
                <code>{appState?.data_dir || "加载中"}</code>
              </p>
            </div>
          </div>
        </div>
      )}

      {probe && (
        <div
          className="modal-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget && busy !== "trust") setProbe(null);
          }}
        >
          <div className="modal" role="dialog" aria-modal="true" aria-labelledby="trust-modal-title">
            <div className="modal-header">
              <h2 id="trust-modal-title">确认 {probe.alias} 的指纹</h2>
              <button
                type="button"
                className="icon-btn"
                aria-label="关闭"
                onClick={() => setProbe(null)}
                disabled={busy === "trust"}
              >
                <X size={16} aria-hidden="true" />
              </button>
            </div>
            <div className="trust-body">
              <p>请通过可信渠道核对服务器指纹。确认后，后续连接会强制校验该密钥。</p>
              <div className="fingerprint-box">{probe.fingerprint}</div>
              <code className="key-line">{probe.host_key_line}</code>
              <div className="modal-actions">
                <button type="button" className="btn ghost" onClick={() => setProbe(null)} disabled={busy === "trust"}>
                  取消
                </button>
                <button type="button" className="btn primary" onClick={trustProbe} disabled={busy === "trust"}>
                  {busy === "trust" ? "保存中…" : "指纹正确，信任"}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
