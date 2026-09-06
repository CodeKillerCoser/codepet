# Harness 接入以正式版本和实际入口验证

## 规则

用户未指定预览版本时，Provider 接入以正式发行版支持的 API 为目标。验证需要同时绑定版本、入口和事件语义；HTTP API 名称中的 V2 不代表插件 API 属于同一发布通道。

## 适用场景

Provider 的 hook / 插件注入、桌宠活动感知、runtime 版本选择，以及上游 SDK 升级。

## 反例

OpenCode `1.18.25` 是正式版，普通 CLI 插件返回 `event` hooks。仅测试另一路径的 `setup(ctx)`，发现缺少 `ctx.event.subscribe` 后要求升级 Beta，会漏掉正式版已有的接入能力。Beta 收到 `session.created` 也不足以宣称正式版任务生命周期已经接通。

## 推荐做法

- 阅读目标正式 tag 的插件 loader、类型定义和事件发布位置；在线最新文档必须核对发布通道。
- 用目标二进制和隔离配置走实际入口。本地模拟模型可以覆盖任务成功、失败和权限等待，避免依赖真实模型服务。
- Schema 中存在路由不等于正式二进制已实现。OpenCode 1.18.25 的 `/api/session/:id/wait` 实际返回 503；health/list smoke 与自行模拟成功的 fixture 都不能证明发送后的完成等待可用。控制入口至少覆盖连续两轮发送、工具审批、终态、历史重载和再次取得交互权。
- 插件只观察、异步投递；任务完成依据正式 session 事件。工具/模型 step 结束不直接等于任务完成，失败后的 idle 不覆盖失败。
- 桌宠全局感知至少验证两个独立 harness 进程；不以 Provider 自有 Remote Server 的事件覆盖范围替代。

## 来源

- [活动订阅实现与修正记录](../10-architecture/provider-hook-observation-proposal.md)
- [OpenCode 历史、创建控件与完成等待修复](../40-runbooks/opencode-history-controls-and-completion.md)
- [OpenCode v1.18.25 插件加载与事件回调](https://github.com/anomalyco/opencode/blob/v1.18.25/packages/opencode/src/plugin/index.ts)
- [正式版 Hook 类型](https://github.com/anomalyco/opencode/blob/v1.18.25/packages/plugin/src/index.ts)

## 验证方式

- `node --test crates/providers/codepet-provider-opencode/tests/observation-plugin.test.mjs`
- `python3 scripts/test_opencode_observation.py --executable /opt/homebrew/bin/opencode`
- `python3 scripts/test_opencode_observation.py --executable /opt/homebrew/bin/opencode --remote-turns`
- `cargo test --manifest-path crates/Cargo.toml -p codepet-host pet_gateway::projection`
- Review 区分正式版实测、fixture 测试及尚未验证的版本/入口；无法接入时先确认适配缺口，不直接将升级 Beta 作为用户前置条件。
