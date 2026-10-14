# AgentUpdate v0.1.0 macOS 使用、恢复与支持范围

目标系统为 macOS 15 或更高版本。首版候选安装包包含 arm64 与 x86_64 两个切片；本机验证为 Apple Silicon 原生与 Rosetta 启动，物理 Intel 与干净 macOS 15 仍待补验，不代表已完成这两类实机验收。

管理 npm 工具需要外部 Node.js ≥22.12 和 npm，且与所管理实例的架构兼容。应用不内置 Node/npm；不满足条件的实例不能执行更新。

## 安装

1. 使用本轮交付目录中的 `AgentUpdate-0.1.0-macos-universal.dmg`，并对照同目录 `SHA256SUMS` 校验；GitHub Release 尚未公开发布。
2. 打开磁盘映像，将 `AgentUpdate.app` 拖入 `/Applications`。
3. 当前候选使用 ad-hoc 签名，按用户要求用于本地使用与审核，尚无 Developer ID 签名/公证。如被 macOS 阻止，核对本轮文件和哈希后在“系统设置 → 隐私与安全性”使用系统提供的“仍要打开”。
4. 应用不需要管理员权限。它只管理当前用户可写的受支持安装。

## 更新 Agent

应用启动后扫描本机，显示工具、实际路径、版本、来源和架构。支持当前用户拥有的 Codex CLI、Gemini CLI、Claude Code CLI npm 实例，以及默认 Claude 用户原生安装与标准 Homebrew Cask。Codex Cask 显示实际版本但暂不自动修改。勾选实例并分别选择目标版本，确认计划后再执行。Claude 还提供“一键升级最新稳定版”，目标按当前来源确定；Homebrew 目录可能晚于官方原生渠道。配置备份不能关闭；备份或恢复材料准备失败时，该项不会修改安装。

“应用版本与更新检测”读取 Codex app、Claude app 和 ChatGPT / 集成 Codex 的本机版本，并显示公开渠道版本、检查时间和官方更新入口。它只打开官方页面或匹配制品，不自动替换桌面应用。详细来源见 [追加功能说明](claude-and-desktop-updates.md)。

批量任务按顺序执行。一项失败会暂停后续项，已完成项保留各自恢复点。更新期间不要同时用另一个进程升级同一 npm 实例。

## 恢复

在历史记录中选择对应事务，先查看恢复预览。若更新后配置发生变化，应用会先保存当前配置副本，再离线恢复旧版本和原配置。恢复失败时不要删除应用数据；按 `docs/rollback.md` 保留恢复点并人工处理。应用异常退出后，重新打开会识别未完成事务，不会自动重装或把中断结果标成成功。

应用数据位于 `~/Library/Application Support/AgentUpdate`，包含数据库、配置快照、旧版包和导出。备份包括 Codex 的 `config.toml` / `auth.json`，以及 Gemini 的 `settings.json` / `oauth_creds.json` / `google_accounts.json` / `mcp-oauth-tokens.json` 等受管文件，可能包含令牌和账号信息。应用不上传这些原始材料；完整导出必须在界面明确确认后才会生成，保管和分享时按敏感材料处理。详见 `docs/backup-recovery-material.md`。

## 卸载

先完成或明确保留所有待恢复事务，再退出应用。将 `/Applications/AgentUpdate.app` 移到废纸篓即可卸载程序。若同时删除 `~/Library/Application Support/AgentUpdate`，所有恢复点和历史会永久删除；在确认不再需要回滚前不要删除该目录。卸载应用不会修改已安装的 Codex 或 Gemini。

## 支持矩阵

| 对象 | v0.1.0 状态 |
| --- | --- |
| Codex CLI / 当前用户 npm | 支持发现、固定版本更新、备份和回滚 |
| Gemini CLI / 当前用户 npm | 支持发现、固定版本更新、备份和回滚 |
| Claude Code CLI / 用户 npm、默认用户原生、标准 Homebrew Cask | 支持发现、最新稳定版更新、配置备份与旧版本恢复；npm 可选指定版本 |
| Codex Homebrew、其他系统级 npm/原生来源 | 读取版本与限制，不自动修改 |
| Codex app、Claude app、ChatGPT | 本机版本与公开渠道检测，提供官方更新入口 |
| Windows / Linux | 未实现 |

## 已知限制

- 当前候选包没有 Developer ID 签名和 Apple 公证，不能作为 Gatekeeper 通过的公开下载。
- 物理 Intel Mac、干净 macOS 15 安装、两种工具的安装包内更新/回滚和安装包卸载尚未在独立测试环境复核。
- 应用只检查自身 GitHub Release 并打开匹配安装包，不自动替换自身。
- 可备份并恢复受管文件中的凭据，但不管理登录认证或 Keychain；IDE 插件与远端服务不在范围内。Claude 备份默认 `.claude` 下的 settings/credentials 文件及 `~/.claude.json`，不支持自定义配置根目录。
- 官方版本查询需要网络；下载校验失败、工具占用、权限不足或磁盘不足时会停止该项并保留恢复材料。
