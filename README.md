# CyberScope

CyberScope 是一个私有 FOFA 资产检索控制台，提供 Web 查询、结果查看和文件导出。前端会嵌入 Rust 后端，生产环境只需运行一个二进制。

## 功能

- 单管理员登录；
- FOFA 查询与返回字段选择；
- 查询状态、取消和结果持久化；
- 结果筛选与资产详情；
- CSV、JSON、TXT 导出。

## 快速开始

推荐使用已启用 Flakes 的 Nix。

```bash
cp .env.example .env
```

在 `.env` 中填写：

```dotenv
FOFA_API_KEY=your-api-key
WEB_ADMIN_PASSWORD=your-password
```

密码至少需要 8 个字符。启动应用：

```bash
nix run
```

访问 <http://127.0.0.1:3000>，默认用户名为 `admin`。

## 文档

- [安装与启动](docs/getting-started.md)
- [配置说明](docs/configuration.md)
- [Web API](docs/api-reference.md)
- [架构说明](docs/web-single-binary-architecture.md)

当前版本仅接入 FOFA，同时最多运行一个查询任务。请仅查询你拥有或已获授权测试的资产。

## 许可证

本项目采用 [MIT License](LICENSE)。
