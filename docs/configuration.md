# 配置说明

> 返回：[项目首页](../README.md)

CyberScope 从环境变量读取配置，并在启动时尝试加载 `.env`。建议从仓库根目录运行。

## 环境变量

| 变量                       | 必填 | 默认值              | 说明                                                |
| -------------------------- | :--: | ------------------- | --------------------------------------------------- |
| `FOFA_API_KEY`             |  是  | —                   | FOFA 官方或兼容端点的 API Key                       |
| `FOFA_API_BASE_URL`        |  否  | `https://fofa.info` | 仅接受没有凭据、参数、片段和额外路径的 HTTPS 根地址 |
| `FOFA_RELAY_QUOTA_ENABLED` |  否  | `false`             | 初始化中转额度客户端；使用 FOFA 官方地址时不能启用  |
| `WEB_ADMIN_USERNAME`       |  否  | `admin`             | 唯一管理员用户名，最长 64 字节且不能包含控制字符    |
| `WEB_ADMIN_PASSWORD`       |  是  | —                   | 管理员密码，至少 8 个字符                           |
| `WEB_BIND_ADDRESS`         |  否  | `127.0.0.1:3000`    | Web 服务监听地址                                    |
| `DATABASE_PATH`            |  否  | `data`              | SQLite 数据目录                                     |

布尔值支持 `true/false`、`1/0`、`yes/no` 和 `on/off`，不区分大小写。

配置模板见 [.env.example](../.env.example)。不要提交真实的 API Key 或管理员密码。

## FOFA 模式

默认地址 `https://fofa.info` 使用官方模式，并要求：

```dotenv
FOFA_RELAY_QUOTA_ENABLED=false
```

兼容中转端点必须提供 HTTPS 根地址。字段、语法和额度规则以服务商的实时文档与 API 响应为准。当前 Web 页面不展示额度信息。

## 登录与数据

- 当前只有一个管理员，没有注册或多用户系统。
- 会话 Cookie 使用 `HttpOnly`、`SameSite=Strict`，有效期 8 小时。
- 会话保存在内存中，退出或服务重启后失效。
- SQLite 默认位于 `data/cyberscope.db`，保存查询任务、表达式和结果，不保存密码、API Key 或会话。
- 重启时，未完成的任务会被标记为失败。

## 部署

应用默认只监听本机且不提供 TLS。远程访问时应使用 HTTPS 反向代理、防火墙和强管理员密码，并限制 SQLite 数据目录的访问权限。
