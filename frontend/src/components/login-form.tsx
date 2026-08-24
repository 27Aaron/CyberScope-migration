import { type FormEvent, useState } from "react";
import { CircleAlert, LogIn } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";

type LoginFormProps = {
  error?: string;
  isPending: boolean;
  onSubmit: (credentials: { username: string; password: string }) => void;
};

export function LoginForm({ error, isPending, onSubmit }: LoginFormProps) {
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    onSubmit({ username: username.trim(), password });
  }

  return (
    <Card>
      <form className="contents" onSubmit={handleSubmit}>
        <CardHeader>
          <CardTitle>登录控制台</CardTitle>
          <CardDescription>
            使用部署时配置的管理员凭据访问资产检索工作台。
          </CardDescription>
        </CardHeader>
        <CardContent>
          <FieldGroup>
            {error ? (
              <Alert variant="destructive">
                <CircleAlert />
                <AlertTitle>无法登录</AlertTitle>
                <AlertDescription>{error}</AlertDescription>
              </Alert>
            ) : null}
            <Field data-invalid={Boolean(error)}>
              <FieldLabel htmlFor="username">用户名</FieldLabel>
              <Input
                id="username"
                name="username"
                autoComplete="username"
                aria-invalid={Boolean(error)}
                disabled={isPending}
                required
                value={username}
                onChange={(event) => setUsername(event.target.value)}
              />
            </Field>
            <Field data-invalid={Boolean(error)}>
              <FieldLabel htmlFor="password">密码</FieldLabel>
              <Input
                id="password"
                name="password"
                type="password"
                autoComplete="current-password"
                aria-invalid={Boolean(error)}
                disabled={isPending}
                required
                value={password}
                onChange={(event) => setPassword(event.target.value)}
              />
            </Field>
          </FieldGroup>
        </CardContent>
        <CardFooter className="flex-col gap-3">
          <Button className="w-full" disabled={isPending} type="submit">
            {isPending ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <LogIn data-icon="inline-start" />
            )}
            {isPending ? "正在验证" : "登录"}
          </Button>
          <FieldDescription>
            登录状态保存在仅限当前站点访问的 HttpOnly 会话 Cookie 中。
          </FieldDescription>
        </CardFooter>
      </form>
    </Card>
  );
}
