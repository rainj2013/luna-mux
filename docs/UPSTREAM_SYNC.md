# Luna Remote 上游同步规则

## 策略

Luna Mux 使用独立的 `origin`。Luna Remote 只配置为名为 `upstream` 的只读拉取远端：禁止向 `upstream` 推送，也禁止整体合并其主分支。

Luna Remote 是实现能力的来源之一，不是 Luna Mux 的产品规范。共同的 Git 历史不意味着 Luna Mux 必须保留 Luna Remote 的架构、导航、术语、数据模型、界面或功能行为。只有符合当前 Luna Mux 设计的能力才可移植；其他改动应在 Luna Mux 边界后适配、重新实现，或标记为 `not-applicable`。

需要持续审阅并选择性移植的基础能力包括：

- SSH、SFTP、文件传输、端口转发、凭据和主机密钥安全；
- 终端解码、输出流控、渲染和平台兼容；
- 数据库正确性以及可复用的迁移修复；
- 构建、依赖、Windows 和 macOS 修复。

不要自动移植 Luna Remote 专属的导航、标签页或 Session 语义、状态所有权、数据关系、品牌、发布元数据、交互流程和产品决策。

## 移植流程

1. 执行 `git fetch upstream`。
2. 审阅上次记录的 SHA 之后的提交。
3. 按当前 Luna Mux 设计将每个提交分类为 `ported`、`adapted`、`pending` 或 `not-applicable`。
4. 可直接复用时执行 `git cherry-pick -x <sha>`。
5. 需要适配时实现等价改动，并在提交信息中加入：

```text
Upstream-Repo: luna-remote
Upstream-Commit: <sha>
Upstream-Module: <module>
Adaptation: modified
```

6. 运行受影响的核心测试和 Luna Mux 回归检查。
7. 在同一次改动中更新下方记录表。

## 同步记录

| 上游提交 | 状态 | 模块 | Luna Mux 提交 | 说明与验证 |
| --- | --- | --- | --- | --- |
| `7ccc61f0bb1ea7ffb74556c1fe207e802bfbfd2f` | `baseline` | 全部 | 继承历史 | Luna Mux 仓库的初始祖先提交 |
