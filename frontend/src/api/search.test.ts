import { describe, expect, it } from "vitest";

import { extractErrorMessage } from "./search";

describe("extractErrorMessage", () => {
  it("解析后端标准错误体 {error: {code, message}}", () => {
    const message = extractErrorMessage(
      { error: { code: "invalid_request", message: "format 必须是 csv、json 或 txt" } },
      400,
    );
    expect(message).toBe("format 必须是 csv、json 或 txt");
  });

  it("兼容顶层 message 的错误体", () => {
    expect(extractErrorMessage({ message: "顶层消息" }, 500)).toBe("顶层消息");
  });

  it("兼容字符串形式的 error 字段", () => {
    expect(extractErrorMessage({ error: "字符串错误" }, 401)).toBe("字符串错误");
  });

  it("body 缺失时回退到状态码占位", () => {
    expect(extractErrorMessage(null, 502)).toBe("请求失败（502）");
    expect(extractErrorMessage(undefined, 503)).toBe("请求失败（503）");
  });

  it("不再把 error 对象渲染成 [object Object]", () => {
    const message = extractErrorMessage({ error: { code: "x" } }, 422);
    expect(message).not.toContain("[object");
  });
});
