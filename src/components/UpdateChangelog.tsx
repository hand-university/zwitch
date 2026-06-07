interface UpdateChangelogProps {
  notes: string;
  compact?: boolean;
  className?: string;
}

export function UpdateChangelog({
  notes,
  compact = false,
  className,
}: UpdateChangelogProps) {
  const trimmed = notes.trim();
  if (!trimmed) return null;

  return (
    <section
      className={
        className ??
        "rounded-lg border border-border/60 bg-secondary/20 px-3 py-2.5"
      }
    >
      <h3 className="mb-1.5 text-xs font-medium text-foreground">更新内容</h3>
      <div
        className={
          compact
            ? "max-h-28 overflow-y-auto whitespace-pre-wrap text-xs leading-relaxed text-muted-foreground"
            : "max-h-40 overflow-y-auto whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground"
        }
      >
        {trimmed}
      </div>
    </section>
  );
}
