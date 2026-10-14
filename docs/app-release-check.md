# 应用自身版本检查（AGE-17）

来源固定为当前 origin 的 GitHub 仓库：zero-times/agentupdate。使用官方 HTTPS Releases API，固定请求头，不发送 Agent 配置、凭据、项目内容或机器路径。后台线程独立检查，不占用更新队列锁；每次请求8秒连接/12秒总超时，网络/5xx最多重试一次。发布列表分页，最多500条，超过范围明确失败；无法确认完整结果时不报告已最新。

筛选非草稿、非预发行的严格 vMAJOR.MINOR.PATCH 标签，按语义化版本选最高稳定版。独立区分暂无发布、更新可用、已最新、本地较新和失败。缓存持久化检查/成功时间、最高稳定发布与 ETag，当前构建版本变化后重新比较。自动检查最多24小时一次；手动检查绕过普通节流，GitHub限流退避仍有效。网络失败保留上次成功结果并明确标旧结果，不伪装已最新。多页数据不依赖第一页 ETag 推断所有页未变。

制品命名协议：AgentUpdate-<version>-macos-aarch64.dmg / x86_64.dmg / universal.dmg；tag/app/version一致。优先当前二进制架构专用包，回退通用包；缺匹配制品明确说明。只接受同一仓库、对应tag、精确命名且已上传的制品URL。应用入口只接收“发布页/制品”选项，从验证后的缓存打开固定链接，无任意URL输入。首期手动下载安装，无静默替换。

界面提供关于、启动后台检查、手动检查、当前/远端版本、检查时间、发布时间、纯文本说明与匹配下载入口。发布说明不作为HTML或可执行内容。

验证包括 v0.9.0/v0.10.0、相等/倒退、草稿/预发行/无效/空列表、架构及通用包、恶意URL、缓存重新比较、网络失败保留旧结果、24小时/手动/限流退避、304与离线有限重试。2026-10-12 的真实 GitHub 请求遇到 RELEASE_RATE_LIMIT，准确返回 failed 和 retryAfter；记录 docs/evidence/age-17-live-release.json。2026-10-13 再次访问固定官方端点，成功返回 no_releases；记录 docs/evidence/age-17-live-release-2026-10-13.json。没有把无发布或失败显示成已是最新版。

AGE-16 已生成 `AgentUpdate-0.1.0-macos-universal.dmg`、真实 `.app.zip`、SHA-256 和制品清单，生产解析/比较测试读取本轮打包生成的 Release fixture，确认 `v0.1.0` 与构建版本一致、aarch64/x86_64 都回退到准确通用 DMG 链接、低版本提示更新而同版本显示已最新。结果见 `docs/evidence/age-16-release-contract.json`。fixture 是本地模拟，不代表真实 Release；签名、公证和实机验收仍按 [版本完成情况检查](v0.1.0-readiness.md) 核对。当前仓库没有真实 Release，因此线上新版与签名下载制品仍待发布后校验；这不要求提前公开发布测试版本。

按用户要求保持进行中，整个版本完成后统一审核。
