import * as React from "react";

import { X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

type SheetProps = { open: boolean; onOpenChange: (open: boolean) => void; children: React.ReactNode };

function Sheet({ open, onOpenChange, children }: SheetProps) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 bg-background/70 backdrop-blur-sm" role="presentation" onMouseDown={() => onOpenChange(false)}>
      <aside
        aria-modal="true"
        className="absolute inset-y-0 right-0 flex w-full max-w-xl flex-col border-l bg-card shadow-2xl"
        role="dialog"
        onMouseDown={(event) => event.stopPropagation()}
      >
        {children}
      </aside>
    </div>
  );
}

function SheetHeader({ className, ...props }: React.ComponentProps<"div">) { return <div className={cn("flex flex-col gap-2 border-b p-6", className)} {...props} />; }
function SheetTitle({ className, ...props }: React.ComponentProps<"h2">) { return <h2 className={cn("text-lg font-semibold tracking-tight", className)} {...props} />; }
function SheetDescription({ className, ...props }: React.ComponentProps<"p">) { return <p className={cn("text-sm text-muted-foreground", className)} {...props} />; }
function SheetContent({ className, ...props }: React.ComponentProps<"div">) { return <div className={cn("flex-1 overflow-y-auto p-6", className)} {...props} />; }
function SheetClose({ onClick }: { onClick: () => void }) { return <Button aria-label="关闭详情" className="absolute right-4 top-4" size="icon" variant="ghost" onClick={onClick}><X data-icon="inline-start" /></Button>; }

export { Sheet, SheetClose, SheetContent, SheetDescription, SheetHeader, SheetTitle };
