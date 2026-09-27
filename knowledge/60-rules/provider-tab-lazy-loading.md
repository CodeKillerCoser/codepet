# 首页 Provider tab 按需加载

## 规则

首页连接设备时，只加载默认选中的 Provider 的项目、普通会话及最近会话第一页；其他 Provider 首次选中时才加载。Provider 描述与连接状态仍可提前获取。每页维持 20 条，最近列表继续按视口自动分页。

## 适用场景

- `codepet-remote/lib/application/sessions/device_session.dart`：连接初始化、tab 切换、Provider 状态更新、设备重连和项目/会话事件均可能触发列表请求，必须统一遵守当前选择。
- `codepet-remote/lib/application/conversations/recent_conversation_controller.dart`：最近列表独立订阅失效事件，需要区分当前激活与后台缓存状态。

## 反例

仅修改首页显示或连接首屏循环，而让 `provider.changed`、项目变更、未知 turn 事件继续为未选中的 Provider 发起列表请求，会绕过懒加载。切换时销毁所有缓存则会重复请求并丢失已加载深度。

## 推荐做法

选中 Provider 后统一调用列表初始化入口，已有第一页与游标继续复用，并用现有请求集合抑制并发重复加载。后台项目/未知 turn 事件保留待刷新信息，最近失效仅标记待刷新；切回后再执行。已在途请求可完成并写入对应 Provider 缓存，但后台不继续发起恢复分页。恢复过程中切走时保留旧最近窗口，避免切回后丢失恢复深度。

重连清除旧游标，只重新加载保留选择的 Provider；当前 Provider 被移除后，加载自动选中的备用 tab。普通聊天、项目内会话与最近列表继续使用各自的分页状态。

## 来源

2026-09-28 用户要求首页改为选中哪个 tab 才加载对应 Provider 项目和会话。源码核对发现原连接路径依次加载所有 Provider 的项目/聊天，并在 online 后为全部 Provider 创建最近控制器。

## 验证方式

2026-09-28 在 `codepet-remote` 执行以下命令，94 项全部通过：

```powershell
flutter test --no-pub test/application/provider_lazy_loading_test.dart test/devices/device_session_test.dart test/application/home_recent_conversations_test.dart test/application/recent_conversation_controller_test.dart test/home/recent_conversation_feed_test.dart test/home/remote_home_screen_test.dart --reporter expanded
git diff --check
flutter analyze --no-pub lib/application/sessions/device_session.dart lib/application/conversations/recent_conversation_controller.dart test/application/provider_lazy_loading_test.dart test/application/recent_conversation_controller_test.dart test/devices/device_session_test.dart
```

静态检查无问题，两个仓库的差异格式检查均通过。

`provider_lazy_loading_test.dart` 覆盖首次选择、切回缓存/游标、后台事件、快速切换、Provider 移除、重连和 360/1000 宽度下真实 tab 点击。最近控制器测试覆盖恢复途中切走不继续请求、切回恢复原深度，防止后台请求与缓存退化。已有首页测试覆盖布局和连接操作可见性。未进行安装包构建或真实 Host/手机联调，实际网络耗时仍需真机验证。
