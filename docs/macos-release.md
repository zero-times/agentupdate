# macOS v0.1.0 发布材料（AGE-16）

版本为 0.1.0，拟发布 tag 为 v0.1.0，目标为 <https://github.com/zero-times/agentupdate/releases/tag/v0.1.0>。[Release 更新说明](releases/v0.1.0.md)、[使用/恢复/卸载说明](macos-v0.1.0-usage.md)用于审核。实际发布需另外授权。

## 制品和版本协议

- 推荐安装包：AgentUpdate-0.1.0-macos-universal.dmg，包含 AgentUpdate.app 和指向 /Applications 的安装快捷方式。
- 附属应用归档：AgentUpdate-0.1.0-macos-universal.app.zip，使用真实 ZIP 格式。
- SHA256SUMS：本轮制品、源码清单、说明和匹配 fixture 的 SHA-256。
- release-manifest.json：版本、拟发布 tag/链接、二进制哈希、两种架构、签名、公证、各文件大小/哈希及验收阻塞。
- source-manifest.json：实际构建输入逐文件哈希。当前仓库没有源码提交，清单记录 sourceCommit: null；公开发布前需提交并从干净检出重新构建。
- release-check-fixture.json：仅供版本检查服务联调，模拟拟发布 tag/链接和已上传制品；不代表真实 Release 已存在。

服务支持 aarch64 / x86_64 专用 DMG，缺少专用包时回退通用包。本轮提供通用包；生产 Rust 测试读取打包生成的 fixture，确认两种架构匹配、版本比较、准确文件名与仓库/tag 下载链接。

## 构建候选

依赖按 package-lock.json 安装；Rust 按 rust-toolchain.toml 配置。仓库根目录执行：

~~~sh
npm ci
npm run package:mac -- --candidate
~~~

每轮输出至独立目录 release-artifacts/v0.1.0/candidate-<UTC时间>/，避免 Vite 清理 dist 或重复构建覆盖旧产物。源码构建前后摘要必须一致。脚本强制正式前端、默认 Rust features、锁定依赖、通用 target；验证 metadata、生产命令、排除自测/注入标记、签名完整性、ZIP 和 DMG 完整性，再生成校验清单。候选采用 ad-hoc，清单明确 distributionReady: false。

本机复用 .validation/toolchain 中 Rust 1.99.0、两种 targets 和依赖缓存，通过 CARGO_TARGET_DIR 解析正式 AgentUpdate.app 并复制后独立打包。验证证据保存在 docs/evidence/age-16-package.json 与对应制品目录。

## 正式签名、公证与装订

用户已接受无 Developer ID 签名的本地包。`npm run package:mac` 默认生成候选包；正式签名模式 `npm run package:mac -- --signed` 要求以下材料，缺失时在构建前报错：

1. 已安装且有私钥的 Developer ID Application 证书。Apple Development / Apple Distribution 无法替代直接分发身份。
2. 用 Apple 官方 xcrun notarytool store-credentials 私密交互保存的 keychain profile，或已由发布负责人配置的 profile。密码不得放入命令参数、Git、日志或看板。

~~~sh
export AGENTUPDATE_SIGNING_IDENTITY='Developer ID Application: <名称> (<TEAM_ID>)'
export AGENTUPDATE_NOTARY_PROFILE='<已配置的 profile>'
npm run package:mac -- --signed
~~~

脚本交由 Tauri 对 app 签名，验证 Developer ID、Hardened Runtime、时间戳和签名完整性；提交 app ZIP，只有 Apple 返回 Accepted 才装订 app 票据，并做 Gatekeeper execute 验证。装订后的 app 重新归档和打包；DMG 再签名、提交公证、装订并验证票据与 Gatekeeper open。最终所有哈希在签名/装订后计算。

本机没有 Developer ID 和 agentupdate 公证 profile，正式流程尚未实际验证。取得材料后须完整重跑并保留 submission id、Accepted 状态、stapler 与 spctl 原始结果；不得仅凭脚本存在宣称公证通过。

## 干净环境验收与发布操作

| 环境 | 必须保存的证据 |
| --- | --- |
| 干净 Apple Silicon / macOS 15+ | 浏览器下载带 quarantine 的最终 DMG、SHA-256、Gatekeeper、安装、窗口打开、两工具更新/回滚、卸载 |
| 物理 Intel / macOS 15+ | 同上，并核验 x86_64 实际运行与 Node/npm 架构 |

测试使用专用用户、隔离 npm prefix 和人工配置：Codex 0.162.0 → 0.162.1 → 0.162.0，Gemini 0.62.0 → 0.63.0 → 0.62.0，保存两项事务 restored、版本与配置/权限恢复结果。卸载应用后确认 CLI 和配置保持可用；应用数据保留，不删除唯一恢复点。Rosetta 启动只作为补充，不能替代 Intel 实机。

本机执行 node scripts/validate-macos-package.mjs <制品目录>：挂载只读 DMG、复制到临时 Applications、检查 ZIP/DMG/app/版本/哈希，以隔离 HOME 启动双架构生产二进制，再移除临时应用。它验证结构与启动存活，不能替代干净系统、Gatekeeper 或两工具交互验收。

正式签名、公证与上述验收完成后，核对同一干净源码提交、所有版本、最终哈希和说明；完成后提交 in_review。创建 tag 和 GitHub Release 需用户另行授权；目标仓库为 zero-times/agentupdate，上传最终 DMG、ZIP、SHA256SUMS 和清单，并抽查版本检查服务的精确链接。

当前未公开发布不影响准备审核；本任务尚有证书、公证、干净双机验收及源码提交阻塞。

2026-10-13 使用要求调整：签名公证不再阻挡本轮本地使用与功能审核；相关材料和独立设备状态继续作为实际限制记录。新增 CLI 和桌面检测功能会重新构建候选，旧 `candidate-20261013T012754075Z` 不包含本轮新增功能。当前包的最终使用结论以新交付记录为准。
