# CyberScope Web API

> 返回：[项目首页](../README.md)

业务接口使用 `/api/v1` 前缀。除健康检查、登录和退出外，其余接口都需要 `cyberscope_session` Cookie。

## 接口

| 方法   | 路径                            | 认证 | 用途                       |
| ------ | ------------------------------- | :--: | -------------------------- |
| `GET`  | `/api/health`                   |  否  | 健康检查                   |
| `POST` | `/api/v1/auth/login`            |  否  | 登录并设置会话 Cookie      |
| `POST` | `/api/v1/auth/logout`           |  否  | 删除当前会话并清除 Cookie  |
| `GET`  | `/api/v1/me`                    |  是  | 获取当前管理员             |
| `GET`  | `/api/v1/fields`                |  是  | 获取当前模式可用的返回字段 |
| `POST` | `/api/v1/searches`              |  是  | 创建查询任务               |
| `GET`  | `/api/v1/searches/{id}`         |  是  | 获取任务状态               |
| `POST` | `/api/v1/searches/{id}/cancel`  |  是  | 取消任务                   |
| `GET`  | `/api/v1/searches/{id}/results` |  是  | 分页读取结果               |
| `GET`  | `/api/v1/searches/{id}/export`  |  是  | 导出结果                   |

查询接口的 JSON 响应通常使用 `data` 包装。应用生成的错误格式为：

```json
{
  "error": {
    "code": "error_code",
    "message": "错误信息"
  }
}
```

## 登录

`POST /api/v1/auth/login`

```json
{
  "username": "admin",
  "password": "your-password"
}
```

成功时返回当前用户并设置 8 小时有效的 HttpOnly Cookie：

```json
{
  "user": {
    "username": "admin"
  }
}
```

凭据错误或会话失效返回 `401`。退出成功返回 `204`。

## 创建查询

`POST /api/v1/searches`

```json
{
  "query": "domain=\"example.com\"",
  "fields": ["host", "ip", "port", "title"],
  "page_size": 100,
  "max_results": 100,
  "full": false
}
```

| 字段          |       默认值 | 规则                                       |
| ------------- | -----------: | ------------------------------------------ |
| `query`       |            — | 必填，由后端校验语法                       |
| `fields`      | 模式默认字段 | 空数组时使用默认字段                       |
| `page_size`   |        `100` | 限制到 `1..=1000`                          |
| `max_results` |        `100` | 限制到 `1..=10000`，最终不超过 `page_size` |
| `full`        |      `false` | 是否请求完整历史数据，取决于上游权限       |

成功返回 `202` 和任务对象。常用字段包括：

- `id`、`status`、`query`、`fields`；
- `matched_size`、`written_rows`；
- `upstream_attempts`、`retries`、`possible_duplicate_charge`；
- `error_code`、`error_message`；
- 创建、开始、完成和更新时间。

任务状态：

```text
queued
running
cancelling
completed
failed
cancelled
```

当前只允许一个任务同时运行；执行槽被占用时返回 `409 job_busy`。

## 结果与导出

`GET /api/v1/searches/{id}/results` 支持：

| 参数       | 默认值 | 规则              |
| ---------- | -----: | ----------------- |
| `page`     |    `1` | 最小为 `1`        |
| `per_page` |  `100` | 限制到 `1..=1000` |

响应中的 `data` 包含 `rows`、`fields`、`total`、`page`、`per_page` 和 `status`。

`GET /api/v1/searches/{id}/export?format=csv|json|txt` 直接返回文件。省略 `format` 时使用 CSV；CSV 包含 UTF-8 BOM。

## 状态码与任务错误

| 状态码 | 含义                       |
| -----: | -------------------------- |
|  `200` | 请求成功                   |
|  `202` | 查询任务已创建             |
|  `204` | 退出成功                   |
|  `400` | 参数或导出格式错误         |
|  `401` | 登录失败或会话失效         |
|  `404` | 路由或任务不存在           |
|  `409` | 任务槽被占用或任务无法取消 |
|  `422` | 查询语法或返回字段无效     |
|  `500` | 本地服务错误               |

FOFA 请求在后台执行，上游错误通常写入失败任务的 `error_code`：

```text
authentication_error
quota_exhausted
rate_limited
upstream_unavailable
upstream_business_error
upstream_protocol_error
export_error
```

API 没有多租户隔离。查询表达式和结果可能包含敏感信息，应限制服务与 SQLite 文件的访问权限。
