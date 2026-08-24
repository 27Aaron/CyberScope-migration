import { type FormEvent, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Field,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";

type LoginFormProps = Omit<React.ComponentProps<"div">, "onSubmit"> & {
  error?: string;
  isPending: boolean;
  onInputChange?: () => void;
  onSubmit: (credentials: { username: string; password: string }) => void;
};

type LoginFieldErrors = {
  username?: string;
  password?: string;
};

export function LoginForm({
  className,
  error,
  isPending,
  onInputChange,
  onSubmit,
  ...props
}: LoginFormProps) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [fieldErrors, setFieldErrors] = useState<LoginFieldErrors>({});
  const passwordError = fieldErrors.password ?? error;

  function clearFieldError(field: keyof LoginFieldErrors) {
    setFieldErrors((current) =>
      current[field] ? { ...current, [field]: undefined } : current,
    );
    onInputChange?.();
  }

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const normalizedUsername = username.trim();
    const nextErrors: LoginFieldErrors = {};

    if (!normalizedUsername) {
      nextErrors.username = "请输入用户名。";
    }
    if (!password) {
      nextErrors.password = "请输入密码。";
    } else if (Array.from(password).length < 8) {
      nextErrors.password = "密码至少需要 8 个字符。";
    }

    setFieldErrors(nextErrors);
    if (nextErrors.username || nextErrors.password) return;

    onSubmit({ username: normalizedUsername, password });
  }

  return (
    <div className={cn("flex flex-col gap-6", className)} {...props}>
      <Card>
        <CardHeader>
          <CardTitle>登录控制台</CardTitle>
        </CardHeader>
        <CardContent>
          <form noValidate onSubmit={handleSubmit}>
            <FieldGroup>
              <Field
                data-disabled={isPending}
                data-invalid={Boolean(fieldErrors.username)}
              >
                <FieldLabel htmlFor="username">用户名</FieldLabel>
                <Input
                  id="username"
                  name="username"
                  autoComplete="username"
                  aria-describedby={
                    fieldErrors.username ? "username-error" : undefined
                  }
                  aria-invalid={Boolean(fieldErrors.username)}
                  disabled={isPending}
                  required
                  value={username}
                  onChange={(event) => {
                    setUsername(event.target.value);
                    clearFieldError("username");
                  }}
                />
                <FieldError id="username-error">
                  {fieldErrors.username}
                </FieldError>
              </Field>
              <Field
                data-disabled={isPending}
                data-invalid={Boolean(passwordError)}
              >
                <FieldLabel htmlFor="password">密码</FieldLabel>
                <Input
                  id="password"
                  name="password"
                  type="password"
                  autoComplete="current-password"
                  aria-describedby={
                    passwordError ? "password-error" : undefined
                  }
                  aria-invalid={Boolean(passwordError)}
                  disabled={isPending}
                  minLength={8}
                  required
                  value={password}
                  onChange={(event) => {
                    setPassword(event.target.value);
                    clearFieldError("password");
                  }}
                />
                <FieldError id="password-error">{passwordError}</FieldError>
              </Field>
              <Field>
                <Button disabled={isPending} type="submit">
                  {isPending ? <Spinner data-icon="inline-start" /> : null}
                  {isPending ? "正在验证" : "登录"}
                </Button>
              </Field>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
