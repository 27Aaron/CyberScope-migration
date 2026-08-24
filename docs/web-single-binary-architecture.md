# CyberScope 架构

> 返回：[项目首页](../README.md)

CyberScope 在开发时分离运行 React 和 Rust，生产构建则把前端嵌入 Rust 二进制。浏览器只访问后端，FOFA API Key 和管理员密码不会进入前端资源。

## 结构

```text
浏览器
  │
  ▼
Axum Web 服务
  ├── 登录与认证中间件
  ├── 查询、结果和导出 API
  ├── React 静态资源
  │
  ├── JobManager ─── FOFA Client ─── FOFA
  │
  └── SearchStore ─── SQLite
```

| 模块       | 职责                                 |
| ---------- | ------------------------------------ |
| `frontend` | 登录、查询表单、任务状态、结果和详情 |
| `web`      | Axum 路由、认证、请求与响应          |
| `auth`     | 单管理员登录和内存会话               |
| `fofa`     | 查询校验、上游请求、重试和错误映射   |
| `jobs`     | 任务状态、并发限制和取消             |
| `searches` | SQLite 迁移、任务与结果存取          |
| `export`   | CSV、JSON、TXT 输出                  |
| `state`    | 组装配置、客户端、数据库和运行时资源 |

## 登录流程

```text
用户名与密码
    │
    ▼
POST /api/v1/auth/login
    │
    ▼
HttpOnly 会话 Cookie
```

会话有效期为 8 小时，只保存在内存中。前端启动时通过 `GET /api/v1/me` 检查会话，查询相关接口统一要求认证。

## 查询流程

```text
POST /api/v1/searches
    │
    ├── 校验查询和返回字段
    ├── 获取任务执行槽
    ├── 写入 SQLite
    └── 启动后台 FOFA 请求
             │
             ├── 更新任务状态
             └── 保存结果或错误
```

任务状态为：

```text
queued → running → completed
                   ├── failed
                   └── cancelling → cancelled
```

当前同时只允许一个查询任务。前端轮询任务状态，任务完成后从 SQLite 读取结果；导出文件按请求即时生成。

## 持久化

默认数据库是 `data/cyberscope.db`：

- `searches` 保存查询、状态、统计、错误和时间；
- `search_results` 保存结果行；
- 密码、API Key 和会话不会写入数据库；
- 重启后未完成的任务会被标记为失败。

## 开发与生产

开发模式运行两个服务：

```text
Vite  127.0.0.1:5173
Axum  127.0.0.1:3000
```

Vite 把 `/api` 代理到 Axum。生产构建流程为：

```text
pnpm build → frontend/dist → Cargo + RustEmbed → cyberscope
```

最终二进制同时提供 API、静态资源和 SPA fallback，运行时不需要 Node.js。

## 安全与限制

- FOFA API 地址必须是 HTTPS 根地址，上游客户端不自动跟随重定向；
- 默认只监听本机，远程访问需要 HTTPS 反向代理和网络访问控制；
- SQLite 包含查询与结果，应限制文件权限；
- 当前只有 FOFA、单管理员和单任务执行槽；
- 尚无注册、RBAC、多租户、分布式任务、批量 Web 入口或查询列表 API。

HTTP 路由和请求字段见 [Web API](api-reference.md)，运行配置见 [配置说明](configuration.md)。
