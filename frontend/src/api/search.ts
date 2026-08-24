export type ApiField = {
  name: string;
  label: string;
  description?: string;
  type?: string;
};

export type SearchRequest = {
  query: string;
  fields: string[];
  page_size: number;
  max_results: number;
  full: boolean;
};

export type SearchTask = {
  id: string;
  status: string;
  query?: string;
  fields?: string[];
  total?: number;
  matched_size?: number;
  written_rows?: number;
  max_results?: number;
  error?: string;
  error_code?: string;
  error_message?: string;
  created_at?: string;
  started_at?: string;
  completed_at?: string;
  [key: string]: unknown;
};

export type SearchResults = {
  rows: Record<string, unknown>[];
  fields: string[];
  total: number;
  page: number;
  per_page: number;
};

type ApiErrorBody = { code?: string; message?: string };
type ApiEnvelope = {
  data?: unknown;
  message?: string;
  error?: string | ApiErrorBody;
  detail?: string;
};

/** Parse the backend error envelope with compatibility fallbacks. */
export function extractErrorMessage(
  payload: (ApiEnvelope & { status?: number }) | null | undefined,
  status?: number,
): string {
  const nested = typeof payload?.error === "object" ? payload.error : undefined;
  return (
    nested?.message ??
    (typeof payload?.error === "string" ? payload.error : undefined) ??
    payload?.message ??
    payload?.detail ??
    `请求失败（${status ?? payload?.status ?? 0}）`
  );
}

const DEFAULT_FIELDS = ["host", "ip", "port", "country_name", "org", "lastupdatetime"];

async function request<T>(input: RequestInfo | URL, init?: RequestInit): Promise<T> {
  const response = await fetch(input, { headers: { "Content-Type": "application/json", ...init?.headers }, ...init });
  const payload = (await response.json().catch(() => null)) as ApiEnvelope | null;
  if (!response.ok) {
    throw new Error(extractErrorMessage(payload, response.status));
  }
  return payload as T;
}

function unwrapData(payload: unknown): unknown {
  if (payload && typeof payload === "object" && "data" in payload) {
    return (payload as { data?: unknown }).data;
  }
  return payload;
}

function toField(value: unknown): ApiField | null {
  if (typeof value === "string") return { name: value, label: value };
  if (!value || typeof value !== "object") return null;
  const item = value as Record<string, unknown>;
  const name = String(item.name ?? item.key ?? item.field ?? item.api_name ?? "");
  if (!name) return null;
  return {
    name,
    label: String(item.label ?? item.title ?? item.display_name ?? name),
    description: typeof item.description === "string" ? item.description : undefined,
    type: typeof item.type === "string" ? item.type : undefined,
  };
}

export async function fetchFields(): Promise<ApiField[]> {
  const payload = await request<unknown>("/api/v1/fields");
  const unwrapped = unwrapData(payload);
  const list = Array.isArray(unwrapped)
    ? unwrapped
    : unwrapped && typeof unwrapped === "object" && Array.isArray((unwrapped as { fields?: unknown[] }).fields)
      ? (unwrapped as { fields: unknown[] }).fields
      : [];
  return list.map(toField).filter((field): field is ApiField => Boolean(field));
}

export async function createSearch(input: SearchRequest): Promise<SearchTask> {
  const payload = await request<{ data?: SearchTask }>("/api/v1/searches", {
    method: "POST",
    body: JSON.stringify(input),
  });
  const task = unwrapData(payload);
  if (!task || typeof task !== "object") throw new Error("创建任务响应缺少任务信息");
  const normalized = task as SearchTask;
  if (!normalized.id) throw new Error("创建任务响应缺少任务 ID");
  return normalized;
}

export async function fetchSearch(id: string): Promise<SearchTask> {
  const payload = await request<unknown>(`/api/v1/searches/${encodeURIComponent(id)}`);
  const task = unwrapData(payload);
  if (!task || typeof task !== "object") throw new Error("任务状态响应格式无效");
  return task as SearchTask;
}

export async function fetchSearchResults(id: string, page: number, perPage: number): Promise<SearchResults> {
  const payload = await request<unknown>(
    `/api/v1/searches/${encodeURIComponent(id)}/results?page=${page}&per_page=${perPage}`,
  );
  const value = unwrapData(payload);
  const data = value && typeof value === "object" ? (value as Record<string, unknown>) : {};
  const rawRows = Array.isArray(value) ? value : (data.results ?? data.rows ?? []);
  const rows = Array.isArray(rawRows)
    ? rawRows.map((row) => {
        if (row && typeof row === "object" && !Array.isArray(row)) return row as Record<string, unknown>;
        return { value: row };
      })
    : [];
  const rawFields = data.fields;
  const fields = Array.isArray(rawFields) ? rawFields.map(String) : rows[0] ? Object.keys(rows[0]) : DEFAULT_FIELDS;
  return {
    rows,
    fields,
    total: Number(data.total ?? data.total_results ?? rows.length),
    page: Number(data.page ?? page),
    per_page: Number(data.per_page ?? data.page_size ?? perPage),
  };
}

export async function cancelSearch(id: string): Promise<SearchTask> {
  const payload = await request<unknown>(`/api/v1/searches/${encodeURIComponent(id)}/cancel`, { method: "POST" });
  const task = unwrapData(payload);
  return (task && typeof task === "object" ? task : { id, status: "cancelled" }) as SearchTask;
}

export async function exportSearch(id: string, format: "csv" | "json" | "txt"): Promise<Blob> {
  const response = await fetch(`/api/v1/searches/${encodeURIComponent(id)}/export?format=${format}`);
  if (!response.ok) {
    const payload = (await response.json().catch(() => null)) as ApiEnvelope | null;
    throw new Error(extractErrorMessage(payload, response.status));
  }
  return response.blob();
}

export { DEFAULT_FIELDS };
