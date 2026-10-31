# SSH MCP

Windows 桌面托盘应用：在 **Rust + Tauri** 单进程内运行 MCP Streamable HTTP 服务，通过 GUI 管理 SSH 主机与加密密码。AI 只需提供已登记的主机别名、主机名或 IP，不需要也拿不到 SSH 密码。

## 主要功能

- Tauri 2 + React 桌面 GUI
- 系统托盘：关闭窗口隐藏，托盘退出才停止 MCP
- 用户登录自启动（`HKCU\...\Run`）
- 单实例运行，重复启动唤出已有窗口
- 进程内 MCP：`http://127.0.0.1:8765/mcp`
- 多主机 SSH 连接池（`russh` + SFTP）
- SSH 主机指纹确认与强制校验
- AES-256-GCM 加密 SSH 密码；主密钥由 Windows DPAPI CurrentUser 保护
- JSON 原子写入、备份与数据目录 ACL
- MCP Bearer Token 鉴权
- 审计日志不记录密码或完整命令正文

## 技术要求

- Windows 10/11
- Node.js 20+
- Rust stable + MSVC 工具链
- WebView2 Runtime（Windows 11 通常已内置）

Cargo 默认路径：`%USERPROFILE%\.cargo\bin`。建议配置国内 crates 镜像（如 rsproxy sparse）。

## 开发

```powershell
.\scripts\setup.ps1
.\scripts\run-dev.ps1
```

分步：

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
npm install
cd src-tauri
cargo test --all-targets
cd ..
npm run build
npm run tauri -- dev
```

检查：

```powershell
.\scripts\check.ps1
```

## GUI 使用

1. 添加主机：别名、主机名/IP、端口、用户名、SSH 密码。
2. 「验证指纹」→ 核对后信任。
3. 「测试连接」验证密码与主机密钥。
4. 在「Agent 接入」复制 Endpoint 与 Token。
5. 关闭窗口进托盘；托盘「退出」才会停止 MCP。

## MCP 接入

```text
http://127.0.0.1:8765/mcp
Authorization: Bearer <GUI 中的 Token>
```

| 工具 | 参数 | 说明 |
|---|---|---|
| `list_hosts` | 无 | 列出已登记主机（无凭据） |
| `ssh_exec` | `host`, `command`, 可选超时/输出上限 | 执行命令 |
| `ssh_read_file` | `host`, `path`, 可选 `max_bytes` | SFTP 读文本 |
| `ssh_list_dir` | `host`, `path` | SFTP 列目录 |

`host` 可以是唯一别名、主机名或 IP。未登记、已禁用或未确认指纹的主机会被拒绝。

冒烟（应用运行中）：

```powershell
.\scripts\mcp_smoke.ps1 health
.\scripts\mcp_smoke.ps1 tools -Token <token>
.\scripts\mcp_smoke.ps1 exec -Token <token> -HostAlias prod-web-1 -Command "hostname"
```

## 数据目录

默认：`%APPDATA%\SSH-MCP`

开发/测试可覆盖：

```powershell
$env:SSH_MCP_DATA_DIR = "D:\projects\ssh-mcp\.dev-data"
```

| 文件 | 说明 |
|---|---|
| `store.json` | 主机元数据与密码密文 |
| `settings.json` | MCP 地址与加密 Token |
| `master.key.dpapi` | DPAPI 保护的 AES 主密钥 |
| `audit.jsonl` | 审计 |
| `logs\ssh-mcp.log` | 运行日志 |

目录会去掉继承 ACL，仅保留当前用户、SYSTEM、Administrators。

## 安全说明

- SSH 密码需可逆解密才能登录，因此加密存储而非哈希。
- GUI 经 Tauri IPC 调后端，不暴露管理 HTTP API。
- MCP 仅监听 `127.0.0.1`，勿直接暴露公网。
- 主机密钥未确认前不会发送密码或执行命令。
- 当前仅支持密码认证；后续可加私钥且不必改 MCP 工具参数。
- 拥有当前 Windows 用户执行权的程序仍可控制该进程——这是用户级桌面常驻方案的边界。

## 构建安装包 / 便携版

```powershell
.\scripts\build-standalone.ps1
.\scripts\build-portable.ps1
.\scripts\smoke-portable.ps1
```

输出大致为：

```text
src-tauri\target\bundle-release\ssh-mcp.exe
src-tauri\target\bundle-release\bundle\nsis\SSH MCP_0.1.0_x64-setup.exe
dist\SSH-MCP-portable\SSH MCP.exe
dist\SSH-MCP_0.1.0_x64-portable.zip
```

便携版是单个原生 exe，不需要 Python 运行时。

## 项目结构

```text
src/                 React GUI
src-tauri/src/       Rust 后端（MCP / SSH / 加密 / 托盘 / 自启动）
scripts/             开发、检查、打包与冒烟脚本
```
