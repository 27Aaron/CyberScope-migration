# FOFA Telegram Bot

一个面向私有白名单用户的 FOFA 查询机器人。它通过 Telegram 接收单条查询或批量 TXT，流式生成 CSV/TXT 文件，并在发送结束后删除临时结果。

> 仅用于查询你拥有或已获授权测试的资产。请遵守 FOFA、Telegram 及所在地的法律和服务条款。

## 当前行为

- `/search <query>` 使用 `/api/v1/search/all`，每条查询默认最多返回并导出 100 条结果。
- `/batch` 使用显式对话选择 CIDR 或完整查询模式；每个输入查询分别遵守结果上限，达到上限后不再继续游标。
- 全局最多运行一个查询任务；Bot 忙时立即拒绝新任务，不建立内存队列。
- 批量任务按 1 次/秒的保守节奏调用上游，并对成功或失败的逻辑请求统一应用 1000 次安全上限。
- 单次上游响应设有 64 MiB 硬上限；结果按 45 MiB 拆分，单任务最多生成 10 个 part，达到上限时安全保留并发送已提交部分。
- `/settings` 汇总显示全部用户设置，并列出快速修改指令；返回字段默认全选，`/fields` 分页展示当前端点文档支持的全部字段（FOFA 官方 51 个、中转站 40 个），点击立即生效，也保留直接传入字段列表的高级用法。全选包含大字段，单请求上限会自动收紧；官方字段的实际可用性取决于 API Key 套餐。用户设置只在当前进程内有效。
- 默认使用 FOFA 官方 API。仅当 `FOFA_API_BASE_URL` 的主机名为 `fofa.info` 时使用官方字段能力；其他 API 根地址按中转站能力处理。只有明确启用后才初始化可选的中转额度客户端。
- 不使用数据库，不保存历史；查询、凭据和资产正文不会写入日志。

这些默认决策补齐了设计文档中尚未定义的 `/search` 路由、全局排队语义和批量文件组织方式。

## Nix 开发环境

项目不依赖系统全局安装的 Rust。进入锁定的开发环境：

```bash
nix develop
```

如果这是一个尚无首次提交的全新仓库，请先将源码加入 Git index 或完成首次提交，再运行上面的命令。不要使用 `nix develop path:.`：Nix 的 path fetcher 会把被 Git 忽略的 `.env` 和 `target/` 一起复制到本机 Nix store。标准 Git flake 模式会排除这些未跟踪/已忽略文件。

也可以直接执行单条命令：

```bash
nix develop -c cargo test --all-targets
nix develop -c cargo clippy --all-targets --all-features -- -D warnings
nix develop -c cargo build --release
```

## 配置

复制示例文件，填写真实凭据：

```bash
cp .env.example .env
```

| 环境变量                    | 说明                                                      |
| --------------------------- | --------------------------------------------------------- |
| `TELEGRAM_BOT_SECRET`       | 从 `@BotFather` 获取的 Bot Token                          |
| `TELEGRAM_ALLOWED_USER_IDS` | 允许访问的 Telegram 数字用户 ID，逗号分隔；空值拒绝所有人 |
| `FOFA_API_KEY`              | FOFA 查询 API Key                                         |
| `FOFA_API_BASE_URL`         | 默认 `https://fofa.info`；可切换到自定义 FOFA 兼容端点     |
| `FOFA_RELAY_QUOTA_ENABLED`  | 仅在使用兼容中转端点时可设为 `true`                       |

程序只接受无凭据、无 query/fragment 和额外路径的 HTTPS API 根地址。真实 `.env` 已被 Git 忽略。

## 运行

```bash
nix develop -c cargo run --release
```

常用命令：

```text
/search title="login" && country="CN"
/batch
/fields ip,port,host,title,link
/settings
/limit 100
/format csv
/full off
/status
/cancel
```

批量 TXT 支持 UTF-8 BOM 与 CRLF，忽略空行和以 `#` 开头的注释。CIDR 模式会严格验证每一行；完整查询模式不会自动猜测或改写语义。

## 验证

默认测试全部使用本地模型或 mock HTTP 服务，不会请求 Telegram/FOFA，也不会消耗额度：

```bash
nix develop -c cargo fmt --all -- --check
nix develop -c cargo clippy --all-targets --all-features -- -D warnings
nix develop -c cargo test --all-targets --all-features
nix flake check
```

接口、字段和安全约束详见 [`docs/telegram-fofa-bot-design.md`](docs/telegram-fofa-bot-design.md)。
