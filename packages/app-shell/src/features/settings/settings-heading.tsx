/** Gives every settings pane a consistent heading and readable measure. */
export function SettingsHeading({ title, description }: { title: string; description: string }) {
  return (
    <header>
      <h2 className="text-lg font-semibold">{title}</h2>
      <p className="mt-1 max-w-2xl text-sm leading-6 text-muted-foreground">{description}</p>
    </header>
  );
}
