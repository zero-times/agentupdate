# Claude CLI 与桌面应用更新检测

2026-10-13 用户追加范围，并接受无 Developer ID 签名的本地候选包。本轮签名公证不作为本地使用与功能审核门槛，尚未执行的公证和独立设备验收继续如实记录。

## Codex CLI 版本修复

原扫描器遇到 Homebrew Cask 或原生来源就提前返回，版本为空；本机 `/opt/homebrew/bin/codex` 指向 Homebrew Cask，因而没有显示实际版本。现在对这些来源也执行隔离的 `--version` 检查，并读取 Mach-O 架构。版本能读取与安装能安全更新分别表示；当前 Codex Cask 仍仅展示，不自动修改。系统级 npm 安装也会核验实际版本。

## Claude Code CLI 一键升级

工作台新增 Claude Code CLI 和“一键升级最新稳定版”入口，固定目标后显示备份计划，确认后执行。所有来源沿用事务、顺序队列、操作锁、强制备份、更新验证和恢复入口。同版本不重复升级；检测到工具仍运行时要求先退出。

| 来源 | 目标渠道与操作 | 恢复材料 |
| --- | --- | --- |
| 当前用户 npm | 官方 `@anthropic-ai/claude-code`；隔离安装后交换包目录 | 完整旧包、依赖、启动链接与配置 |
| 用户原生 | 默认 `~/.local/bin/claude` 指向 `~/.local/share/claude/versions/<version>`；Claude 官方 `latest` 与版本 manifest、架构 SHA-256，隔离验证后写入新版本并切换链接 | 完整版本目录、原启动链接与配置 |
| 标准 Homebrew Cask | `/opt/homebrew` 或 `/usr/local` 中 `claude-code`；公开 cask 目录固定目标与官方原生制品 SHA-256，再使用 `brew upgrade --cask claude-code` | 完整专用 Caskroom 目录（含元数据）、原启动链接与配置 |

Homebrew 目录可能晚于官方原生渠道；界面说明来源，不能把不同来源的版本目录混用。Homebrew 定义若包含不支持的安装器或生命周期脚本会拒绝生成目标；不会使用 sudo、执行其他 cask 升级或改动 CLI 登录认证。运行时升级出现错误会保留恢复点，实际版本、架构和二进制哈希未核验一致前不报成功。

Claude 配置备份范围：`~/.claude/settings.json`、`settings.local.json`、`.credentials.json`，以及 `~/.claude.json`。包含凭据的文件仅保存在私有恢复目录，不进入诊断日志，不收集会话与项目内容。链接离开 Claude 范围会拒绝。更新后配置有变化，恢复前保存当前副本；受管文件及原有缺失状态恢复，无关新增文件保留。非默认 `CLAUDE_CONFIG_DIR` 暂停止更新并解释原因。

## 桌面应用

从 `/Applications` 和 `~/Applications` 的顶层 app 读取 `Info.plist`，按 bundle ID 匹配，支持改名与多个副本，不启动应用读取版本。

| 产品 | 身份 | 公开渠道 |
| --- | --- | --- |
| 独立 Codex app | `com.openai.codex` | Homebrew `codex-app` 目录与 OpenAI 官方制品 URL |
| Claude app（含 Claude Code 功能） | `com.anthropic.claudefordesktop` | Claude 官方 `RELEASES.json` 稳定更新文档 |
| ChatGPT / 集成 Codex | `com.openai.chat`；或 `com.openai.codex` 且 bundle 名称/可执行文件标识为 ChatGPT | Homebrew `chatgpt` 目录与 OpenAI 官方制品 URL |

各产品独立比较数字版本，显示当前版本、公开版本、检查时间、未安装/可更新/渠道版本一致/本机较新/无法比较/检查失败。失败不显示最新版；目录来源在界面说明，公开目录不保证账户或分批发布资格。Intel 不推荐只含 arm64 的下载链接。按钮只打开受限的官方入口或经来源与架构核验的制品 URL，不自动替换运行中的应用。

网络请求读取环境或 macOS 系统 HTTPS 代理，保持超时和响应上限；检测不上传本机配置和凭据。正式源码及新增命令仍不提供任意进程或文件访问接口。

## v0.2.5 Homebrew 目录过期修复

真实失败记录中，应用查询的公开 cask 为 2.1.292，Homebrew 缓存仍为 2.1.287；禁用自动更新后 `brew upgrade` 返回成功但未升级，最终报 `ARTIFACT_INTEGRITY`。现在使用仅针对目标 cask 的 `upgrade --cask --dry-run` 刷新 API 元数据（`HOMEBREW_FORCE_API_AUTO_UPDATE=1`、`HOMEBREW_API_AUTO_UPDATE_SECS=0`），保持 Homebrew 自身自动更新关闭。随后读取本地 cask，核对目标版本、架构对应 URL、SHA-256 与受支持的安装动作。安装前再次核对，元数据不一致时以 `BREW_CATALOG_CHANGED` 在准备阶段停止。刷新失败提示 `BREW_REFRESH`，原安装不变。安装仍使用原有备份、精确文件哈希与实际版本验证；Codex Homebrew 使用同一路径。先前已进入待恢复的任务应先回滚，再重新检测。
