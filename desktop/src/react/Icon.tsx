import { ICON, type IconName } from "../icons";

/** Static stroke icon from the shared set; size via tailwind on the span. */
export function Icon({ name, className = "size-3.5" }: { name: IconName; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={`inline-flex shrink-0 [&>svg]:block [&>svg]:h-full [&>svg]:w-full ${className}`}
      dangerouslySetInnerHTML={{ __html: ICON[name] }}
    />
  );
}
