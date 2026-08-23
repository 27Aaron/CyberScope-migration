import type * as React from "react";

import { cn } from "@/lib/utils";

function Alert({ className, variant = "default", ...props }: React.ComponentProps<"div"> & { variant?: "default" | "destructive" }) {
  return (
    <div
      role="alert"
      className={cn(
        "flex gap-3 rounded-lg border p-3 text-sm",
        variant === "destructive" ? "border-destructive/40 bg-destructive/10 text-destructive" : "bg-muted/40 text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

function AlertTitle({ className, ...props }: React.ComponentProps<"h5">) {
  return <h5 className={cn("font-medium text-foreground", className)} {...props} />;
}

function AlertDescription({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("text-xs leading-5", className)} {...props} />;
}

export { Alert, AlertDescription, AlertTitle };
