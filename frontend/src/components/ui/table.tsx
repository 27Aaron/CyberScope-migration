import * as React from "react";

import { cn } from "@/lib/utils";

function Table({ className, ...props }: React.ComponentProps<"table">) {
  return <div className="relative w-full overflow-auto"><table className={cn("w-full caption-bottom text-sm", className)} {...props} /></div>;
}
function TableHeader({ className, ...props }: React.ComponentProps<"thead">) { return <thead className={cn("border-b border-border", className)} {...props} />; }
function TableBody({ className, ...props }: React.ComponentProps<"tbody">) { return <tbody className={cn("[&_tr:last-child]:border-0", className)} {...props} />; }
function TableRow({ className, ...props }: React.ComponentProps<"tr">) { return <tr className={cn("border-b border-border/70 transition-colors hover:bg-muted/30", className)} {...props} />; }
function TableHead({ className, ...props }: React.ComponentProps<"th">) { return <th className={cn("h-10 whitespace-nowrap px-4 text-left align-middle text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground", className)} {...props} />; }
function TableCell({ className, ...props }: React.ComponentProps<"td">) { return <td className={cn("max-w-64 px-4 py-3 align-middle", className)} {...props} />; }

export { Table, TableBody, TableCell, TableHead, TableHeader, TableRow };
