import { extractErrorMessage } from "@/api/search";

export type SessionUser = {
  username: string;
};

export type Session = {
  user: SessionUser;
};

type ApiErrorBody = {
  error?: string | { code?: string; message?: string };
  message?: string;
  detail?: string;
};

async function parseError(response: Response): Promise<Error> {
  const payload = (await response.json().catch(() => null)) as ApiErrorBody | null;
  return new Error(extractErrorMessage(payload, response.status));
}

export async function fetchSession(): Promise<Session | null> {
  const response = await fetch("/api/v1/me", {
    headers: { Accept: "application/json" },
  });
  if (response.status === 401) return null;
  if (!response.ok) throw await parseError(response);
  return response.json() as Promise<Session>;
}

export async function login(input: {
  username: string;
  password: string;
}): Promise<Session> {
  const response = await fetch("/api/v1/auth/login", {
    method: "POST",
    headers: { "Content-Type": "application/json", Accept: "application/json" },
    body: JSON.stringify(input),
  });
  if (response.status === 401) throw new Error("账号或密码错误");
  if (!response.ok) throw await parseError(response);
  return response.json() as Promise<Session>;
}

export async function logout(): Promise<void> {
  const response = await fetch("/api/v1/auth/logout", { method: "POST" });
  if (!response.ok) throw await parseError(response);
}
