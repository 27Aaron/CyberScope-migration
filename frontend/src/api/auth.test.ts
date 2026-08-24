import { afterEach, describe, expect, it, vi } from "vitest";

import { fetchSession, login, logout } from "./auth";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("auth api", () => {
  it("把未登录的 me 响应归一化为 null", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(new Response(null, { status: 401 })),
    );

    await expect(fetchSession()).resolves.toBeNull();
  });

  it("登录成功后返回当前管理员", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ user: { username: "admin" } }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(
      login({ username: "admin", password: "a secure password" }),
    ).resolves.toEqual({ user: { username: "admin" } });
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/v1/auth/login",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("展示后端标准登录错误", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: { code: "unauthorized", message: "用户名或密码错误" },
          }),
          {
            status: 401,
            headers: { "Content-Type": "application/json" },
          },
        ),
      ),
    );

    await expect(
      login({ username: "admin", password: "wrong" }),
    ).rejects.toThrow("用户名或密码错误");
  });

  it("退出使用 POST 且接受 204", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await expect(logout()).resolves.toBeUndefined();
    expect(fetchMock).toHaveBeenCalledWith("/api/v1/auth/logout", {
      method: "POST",
    });
  });
});
