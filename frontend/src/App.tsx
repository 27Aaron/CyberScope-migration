import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Activity,
  Check,
  ChevronRight,
  Download,
  Filter,
  LoaderCircle,
  Play,
  RefreshCw,
  SlidersHorizontal,
  X,
} from "lucide-react";

import { fetchHealth } from "@/api/health";
import {
  DEFAULT_FIELDS,
  cancelSearch,
  createSearch,
  exportSearch,
  fetchFields,
  fetchSearch,
  fetchSearchResults,
  type ApiField,
  type SearchTask,
} from "@/api/search";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Field, FieldDescription, FieldError, FieldGroup, FieldLabel } from "@/components/ui/field";
import { Separator } from "@/components/ui/separator";
import { Sheet, SheetClose, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";

type FilterItem = { field: string; value: string };

const ACTIVE_STATUSES = new Set(["queued", "running", "cancelling"]);

function escapeQueryValue(value: string) {
  return value.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
}

function withFilters(query: string, filters: FilterItem[]) {
  return [query.trim(), ...filters.map((item) => `${item.field}="${escapeQueryValue(item.value)}"`)]
    .filter(Boolean)
    .join(" && ");
}

function statusLabel(status?: string) {
  switch (status) {
    case "queued": return "排队中";
    case "running": return "检索中";
    case "cancelling": return "正在取消";
    case "completed": return "已完成";
    case "cancelled": return "已取消";
    case "failed": return "失败";
    default: return "未开始";
  }
}

function statusVariant(status?: string): "default" | "secondary" | "outline" {
  if (status === "failed") return "outline";
  if (status === "completed") return "secondary";
  return "default";
}

function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
}

function FieldPicker({ fields, selected, onToggle }: { fields: ApiField[]; selected: string[]; onToggle: (field: string) => void }) {
  const visibleFields = [...fields.filter((field) => selected.includes(field.name)), ...fields.filter((field) => !selected.includes(field.name))].slice(0, 24);
  return (
    <Field>
      <FieldLabel>返回字段</FieldLabel>
      <div className="flex flex-wrap gap-2">
        {visibleFields.map((field) => {
          const active = selected.includes(field.name);
          return (
            <Button
              key={field.name}
              aria-pressed={active}
              size="sm"
              variant={active ? "secondary" : "outline"}
              onClick={() => onToggle(field.name)}
            >
              {active ? <Check data-icon="inline-start" /> : null}
              {field.label}
            </Button>
          );
        })}
      </div>
      <FieldDescription>已选择 {selected.length} 个字段。大字段可能降低单次返回上限。</FieldDescription>
    </Field>
  );
}

function TaskSummary({ task, onCancel, cancelling }: { task?: SearchTask; onCancel: () => void; cancelling: boolean }) {
  if (!task) return null;
  const active = ACTIVE_STATUSES.has(task.status);
  return (
    <div className="flex flex-wrap items-center gap-3 rounded-lg border bg-muted/30 p-3">
      <Badge variant={statusVariant(task.status)}>
        {active ? <LoaderCircle className="animate-spin" data-icon="inline-start" /> : null}
        {statusLabel(task.status)}
      </Badge>
      <span className="text-sm text-muted-foreground">
        {task.written_rows ?? 0} 条结果
        {task.matched_size !== undefined && task.matched_size !== null ? ` / 命中 ${task.matched_size}` : ""}
      </span>
      <span className="font-mono text-xs text-muted-foreground">{task.id}</span>
      {active ? (
        <Button className="ml-auto" disabled={cancelling} size="sm" variant="outline" onClick={onCancel}>
          {cancelling ? <LoaderCircle className="animate-spin" data-icon="inline-start" /> : <X data-icon="inline-start" />}
          取消任务
        </Button>
      ) : null}
    </div>
  );
}

function App() {
  const queryClient = useQueryClient();
  const [query, setQuery] = useState("");
  const [filters, setFilters] = useState<FilterItem[]>([]);
  const [selectedFields, setSelectedFields] = useState<string[]>(DEFAULT_FIELDS);
  const [taskId, setTaskId] = useState<string>();
  const [selectedRow, setSelectedRow] = useState<Record<string, unknown>>();
  const [page, setPage] = useState(1);
  const [formError, setFormError] = useState<string>();

  const health = useQuery({ queryKey: ["health"], queryFn: fetchHealth, refetchInterval: 30_000 });
  const fieldsQuery = useQuery({ queryKey: ["fields"], queryFn: fetchFields, staleTime: 300_000 });
  const taskQuery = useQuery({
    queryKey: ["search", taskId],
    queryFn: () => fetchSearch(taskId as string),
    enabled: Boolean(taskId),
    refetchInterval: (queryState) => (ACTIVE_STATUSES.has(queryState.state.data?.status ?? "") ? 1_200 : false),
  });
  const resultsQuery = useQuery({
    queryKey: ["search-results", taskId, page],
    queryFn: () => fetchSearchResults(taskId as string, page, 50),
    enabled: Boolean(taskId && taskQuery.data) && !ACTIVE_STATUSES.has(taskQuery.data?.status ?? ""),
  });

  const createMutation = useMutation({
    mutationFn: createSearch,
    onSuccess: (task) => {
      setTaskId(task.id);
      setPage(1);
      setSelectedRow(undefined);
      setFormError(undefined);
    },
    onError: (error) => setFormError(error instanceof Error ? error.message : "创建查询任务失败"),
  });
  const cancelMutation = useMutation({
    mutationFn: cancelSearch,
    onSuccess: (task) => {
      queryClient.setQueryData(["search", taskId], task);
      void queryClient.invalidateQueries({ queryKey: ["search-results", taskId] });
    },
    onError: (error) => setFormError(error instanceof Error ? error.message : "取消任务失败"),
  });

  const availableFields = fieldsQuery.data ?? DEFAULT_FIELDS.map((name) => ({ name, label: name }));
  const effectiveQuery = useMemo(() => withFilters(query, filters), [filters, query]);
  const task = taskQuery.data;
  const results = resultsQuery.data;

  function toggleField(field: string) {
    setSelectedFields((current) => {
      if (current.includes(field)) return current.length === 1 ? current : current.filter((value) => value !== field);
      return [...current, field];
    });
  }

  function addFilter(field: string, value: unknown) {
    if (typeof value !== "string" || !value.trim()) return;
    setFilters((current) => current.some((item) => item.field === field && item.value === value)
      ? current
      : [...current, { field, value }]);
  }

  function submitSearch() {
    if (!query.trim()) return setFormError("请输入 FOFA 查询语句");
    if (!selectedFields.length) return setFormError("至少选择一个返回字段");
    createMutation.mutate({ query: effectiveQuery, fields: selectedFields, page_size: 100, max_results: 100, full: false });
  }

  async function handleExport(format: "csv" | "json" | "txt") {
    if (!taskId) return;
    try {
      downloadBlob(await exportSearch(taskId, format), `fofa-search-${taskId}.${format}`);
    } catch (error) {
      setFormError(error instanceof Error ? error.message : "导出失败");
    }
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <div className="mx-auto flex min-h-screen max-w-[1500px] flex-col gap-6 px-4 py-6 lg:px-8">
        <header className="flex flex-col gap-4 border-b pb-6 md:flex-row md:items-end md:justify-between">
          <div className="flex flex-col gap-3">
            <p className="text-xs font-medium uppercase tracking-[0.24em] text-muted-foreground">Asset intelligence</p>
            <div className="flex flex-wrap items-center gap-3">
              <h1 className="text-3xl font-semibold tracking-tight">CyberScope</h1>
              <Badge variant="secondary">Private deployment</Badge>
            </div>
            <p className="max-w-2xl text-sm leading-6 text-muted-foreground">面向授权资产的 FOFA 检索工作台。查询、筛选、结果详情和文件导出均由 Rust 后端处理。</p>
          </div>
          <Badge variant={health.isSuccess ? "secondary" : "outline"}>
            <Activity data-icon="inline-start" />
            {health.isLoading ? "检查服务中" : health.isSuccess ? "Web API 在线" : "Web API 未连接"}
          </Badge>
        </header>

        <section className="grid gap-6 xl:grid-cols-[minmax(0,25rem)_minmax(0,1fr)]">
          <Card className="h-fit">
            <CardHeader>
              <CardTitle>开始检索</CardTitle>
              <CardDescription>输入 FOFA 查询语句，再从结果中点击字段值继续收窄范围。</CardDescription>
            </CardHeader>
            <CardContent>
              <FieldGroup>
                <Field data-invalid={Boolean(formError)}>
                  <FieldLabel htmlFor="fofa-query">FOFA 查询语句</FieldLabel>
                  <Textarea
                    aria-invalid={Boolean(formError)}
                    className="min-h-28"
                    id="fofa-query"
                    placeholder={'例如：title="管理后台" && country="CN"'}
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                  />
                  <FieldDescription>仅用于已授权资产检索。附加筛选会显示在查询预览中。</FieldDescription>
                  {formError ? <FieldError>{formError}</FieldError> : null}
                </Field>
                <FieldPicker fields={availableFields} selected={selectedFields} onToggle={toggleField} />
                {filters.length ? (
                  <Field>
                    <FieldLabel>附加筛选</FieldLabel>
                    <div className="flex flex-wrap gap-2">
                      {filters.map((item) => (
                        <Badge key={`${item.field}:${item.value}`} variant="secondary">
                          <Filter data-icon="inline-start" />
                          {item.field}={item.value}
                          <button aria-label={`移除 ${item.field} 筛选`} className="rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring" type="button" onClick={() => setFilters((current) => current.filter((value) => value !== item))}>
                            <X data-icon="inline-end" />
                          </button>
                        </Badge>
                      ))}
                    </div>
                  </Field>
                ) : null}
                <div className="flex flex-wrap items-center gap-2">
                  <Button disabled={createMutation.isPending} onClick={submitSearch}>
                    {createMutation.isPending ? <LoaderCircle className="animate-spin" data-icon="inline-start" /> : <Play data-icon="inline-start" />}
                    开始检索
                  </Button>
                  <Button disabled={createMutation.isPending} variant="outline" onClick={() => { setQuery(""); setFilters([]); setFormError(undefined); }}>
                    清空
                  </Button>
                </div>
              </FieldGroup>
            </CardContent>
          </Card>

          <section className="flex min-w-0 flex-col gap-6">
            <Card>
              <CardHeader className="flex-row items-start justify-between gap-4">
                <div className="flex min-w-0 flex-col gap-2">
                  <CardTitle>查询任务</CardTitle>
                  <CardDescription className="max-w-3xl break-all font-mono text-xs">{effectiveQuery || "尚未提交查询"}</CardDescription>
                </div>
                {task ? <Button aria-label="刷新任务" size="icon" variant="ghost" onClick={() => void taskQuery.refetch()}><RefreshCw data-icon="inline-start" /></Button> : null}
              </CardHeader>
              <CardContent className="flex flex-col gap-4">
                <TaskSummary task={task} cancelling={cancelMutation.isPending} onCancel={() => taskId && cancelMutation.mutate(taskId)} />
                {task?.error_message ? <Alert variant="destructive"><AlertTitle>查询失败</AlertTitle><AlertDescription>{task.error_message}</AlertDescription></Alert> : null}
                {!task && !createMutation.isPending ? <Empty><EmptyMedia><SlidersHorizontal /></EmptyMedia><EmptyTitle>等待一次检索</EmptyTitle><EmptyDescription>提交查询后，这里会显示任务进度、结果表格和导出操作。</EmptyDescription></Empty> : null}
                {createMutation.isPending ? <Skeleton className="h-16 w-full" /> : null}
              </CardContent>
            </Card>

            {resultsQuery.isError ? <Alert variant="destructive"><AlertTitle>结果加载失败</AlertTitle><AlertDescription>{resultsQuery.error instanceof Error ? resultsQuery.error.message : "无法读取结果"}</AlertDescription></Alert> : null}
            {resultsQuery.isLoading ? <Skeleton className="h-72 w-full" /> : null}
            {results && !resultsQuery.isLoading ? (
              <Card>
                <CardHeader className="flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
                  <div className="flex flex-col gap-2"><CardTitle>资产结果</CardTitle><CardDescription>第 {results.page} 页 · 共 {results.total} 条已保存结果</CardDescription></div>
                  <div className="flex flex-wrap gap-2">
                    {(["csv", "json", "txt"] as const).map((format) => <Button key={format} size="sm" variant="outline" onClick={() => void handleExport(format)}><Download data-icon="inline-start" />{format.toUpperCase()}</Button>)}
                  </div>
                </CardHeader>
                <CardContent className="flex flex-col gap-4">
                  {results.rows.length ? (
                    <>
                      <Table>
                        <TableHeader><TableRow>{results.fields.map((field) => <TableHead key={field}>{field}</TableHead>)}<TableHead>详情</TableHead></TableRow></TableHeader>
                        <TableBody>
                          {results.rows.map((row, index) => <TableRow key={`${String(row.fid ?? row.host ?? row.ip ?? index)}-${index}`}>
                            {results.fields.map((field) => {
                              const value = row[field];
                              const display = value === null || value === undefined ? "-" : String(value);
                              return <TableCell key={field}><button className="max-w-64 truncate text-left font-mono text-xs text-foreground underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" type="button" onClick={() => addFilter(field, value)}>{display}</button></TableCell>;
                            })}
                            <TableCell><Button size="sm" variant="ghost" onClick={() => setSelectedRow(row)}>查看 <ChevronRight data-icon="inline-end" /></Button></TableCell>
                          </TableRow>)}
                        </TableBody>
                      </Table>
                      <Separator />
                      <div className="flex items-center justify-between gap-4"><p className="text-xs text-muted-foreground">点击单元格值即可追加等值筛选。</p><div className="flex gap-2"><Button disabled={page <= 1} size="sm" variant="outline" onClick={() => setPage((value) => Math.max(1, value - 1))}>上一页</Button><Button disabled={results.rows.length < results.per_page} size="sm" variant="outline" onClick={() => setPage((value) => value + 1)}>下一页</Button></div></div>
                    </>
                  ) : <Empty><EmptyMedia><Activity /></EmptyMedia><EmptyTitle>没有结果</EmptyTitle><EmptyDescription>任务已完成，但当前查询没有返回可展示的资产。</EmptyDescription></Empty>}
                </CardContent>
              </Card>
            ) : null}
          </section>
        </section>
      </div>

      <Sheet open={Boolean(selectedRow)} onOpenChange={(open) => !open && setSelectedRow(undefined)}>
        <SheetHeader><SheetTitle>资产详情</SheetTitle><SheetDescription>当前结果的完整字段值。</SheetDescription><SheetClose onClick={() => setSelectedRow(undefined)} /></SheetHeader>
        <SheetContent className="flex flex-col gap-4">{selectedRow ? Object.entries(selectedRow).map(([key, value]) => <div key={key} className="flex flex-col gap-1"><span className="text-xs font-medium uppercase tracking-[0.12em] text-muted-foreground">{key}</span><code className="break-all rounded-md bg-muted p-2 text-xs">{value === null || value === undefined ? "-" : String(value)}</code></div>) : null}</SheetContent>
      </Sheet>
    </main>
  );
}

export default App;
