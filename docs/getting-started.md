# 安装与启动

> 返回：[项目首页](../README.md)

## Nix 运行

复制配置并填写必填项，变量说明见 [配置说明](configuration.md)。

```bash
cp .env.example .env
nix run
```

启动后访问 <http://127.0.0.1:3000>，默认用户名为 `admin`。

## 开发模式

首次安装前端依赖：

```bash
nix develop -c pnpm --dir frontend install --frozen-lockfile
```

分别启动后端和前端：

```bash
nix develop -c cargo run --manifest-path backend/Cargo.toml
```

```bash
nix develop -c pnpm --dir frontend dev
```

开发时访问 <http://127.0.0.1:5173>；Vite 会把 `/api` 代理到默认后端 `127.0.0.1:3000`。

## 手动构建

不使用 Nix 时，需要支持 Rust 2024 edition 的 Rust 工具链、Node.js 22 和 pnpm 11。

```bash
pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend build
cargo build --release --manifest-path backend/Cargo.toml
./backend/target/release/cyberscope
```

必须先构建前端，Cargo 才能把 `frontend/dist/` 嵌入二进制。

## 检查

```bash
nix develop -c cargo fmt --manifest-path backend/Cargo.toml --all --check
nix develop -c cargo test --manifest-path backend/Cargo.toml --all-targets --all-features
nix develop -c pnpm --dir frontend typecheck
nix develop -c pnpm --dir frontend test
nix develop -c pnpm --dir frontend build
nix flake check
```

## 常见问题

| 问题               | 处理方式                                                                    |
| ------------------ | --------------------------------------------------------------------------- |
| 缺少必填环境变量   | 从仓库根目录启动，并检查 `.env` 中的 `FOFA_API_KEY` 和 `WEB_ADMIN_PASSWORD` |
| 密码配置无效       | `WEB_ADMIN_PASSWORD` 至少需要 8 个字符                                      |
| `3000` 端口被占用  | 修改 `WEB_BIND_ADDRESS`；开发模式还要同步修改 Vite 代理目标                 |
| 前端 API 请求失败  | 确认后端已启动，并访问 <http://127.0.0.1:3000/api/health>                   |
| 重启后需要重新登录 | 会话只保存在内存中，服务重启后会失效                                        |
