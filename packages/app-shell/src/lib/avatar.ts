/** A paired background + foreground for a colored initials avatar. */
export interface AvatarColor {
  /** Tailwind background utility for the avatar chip. */
  bg: string;
  /** Tailwind text utility for the initials, a darker shade of the same hue. */
  fg: string;
}

// A fixed palette of solid backgrounds paired with a darker shade of the same
// hue for the initials. The class strings are static so Tailwind v4 generates
// them; the actual hue is selected deterministically from the name.
const AVATAR_PALETTE: readonly AvatarColor[] = [
  { bg: "bg-blue-500", fg: "text-blue-900" },
  { bg: "bg-emerald-500", fg: "text-emerald-900" },
  { bg: "bg-violet-500", fg: "text-violet-900" },
  { bg: "bg-amber-500", fg: "text-amber-900" },
  { bg: "bg-rose-500", fg: "text-rose-900" },
  { bg: "bg-teal-500", fg: "text-teal-900" },
  { bg: "bg-indigo-500", fg: "text-indigo-900" },
  { bg: "bg-orange-500", fg: "text-orange-900" },
];

/**
 * Picks a stable palette color for the given name so the same user always
 * renders the same avatar color across sessions.
 */
export function getAvatarColor(name: string): AvatarColor {
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = (hash * 31 + name.charCodeAt(i)) | 0;
  }
  return AVATAR_PALETTE[Math.abs(hash) % AVATAR_PALETTE.length]!;
}

/**
 * Extracts up to two initials from a display name.
 * "Eric Wang" -> "EW", "Ada" -> "A", "" -> "".
 */
export function getInitials(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return "";
  const parts = trimmed.split(/\s+/).filter(Boolean);
  const first = parts[0] ?? "";
  // Use the last token for the second initial so "Eric Q. Wang" -> "EW".
  const second = parts.length > 1 ? parts[parts.length - 1] : "";
  return (first.charAt(0) + second.charAt(0)).toUpperCase();
}
