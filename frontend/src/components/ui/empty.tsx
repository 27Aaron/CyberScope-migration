import type * as React from "react";

import { cn } from "@/lib/utils";

function Empty({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("flex min-h-36 flex-col items-center justify-center gap-2 rounded-lg border border-dashed p-6 text-center", className)} {...props} />;
}

function EmptyMedia({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("flex size-10 items-center justify-center rounded-full bg-muted text-muted-foreground", className)} {...props} />;
}

function EmptyTitle({ className, ...props }: React.ComponentProps<"h3">) {
  return <h3 className={cn("text-sm font-semibold", className)} {...props} />;
}

function EmptyDescription({ className, ...props }: React.ComponentProps<"p">) {
  return <p className={cn("max-w-md text-xs leading-5 text-muted-foreground", className)} {...props} />;
}

export { Empty, EmptyDescription, EmptyMedia, EmptyTitle };
