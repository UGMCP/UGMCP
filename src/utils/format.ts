import type { PublicUser } from "../types";

export function displayName(user: PublicUser | null | undefined): string {
  if (!user) return "";
  return user.name || user.username || user.nickname || user.email || "Account";
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean).slice(0, 2);
  const letters = parts.map((part) => part[0]?.toUpperCase() ?? "").join("");
  return letters || "•";
}

export function shortPath(path: string): string {
  return path
    .replace(/^\/home\/[^/]+/, "~")
    .replace(/^\/Users\/[^/]+/, "~")
    .replace(/^\/root\b/, "~")
    .replace(/^[A-Za-z]:\\Users\\[^\\]+/i, "~")
    .replace(/^[A-Za-z]:\/Users\/[^/]+/i, "~");
}

export function statusLabel(state: string): string {
  switch (state) {
    case "ONLINE":
      return "online";
    case "OFFLINE":
      return "offline";
    case "DEGRADED":
      return "degraded";
    case "ERROR":
      return "error";
    default:
      return "connecting";
  }
}

export function statusClass(state: string): string {
  switch (state) {
    case "ONLINE":
      return "online";
    case "DEGRADED":
    case "ERROR":
      return "degraded";
    case "CHECKING":
      return "connecting";
    default:
      return "offline";
  }
}

export function osc7Path(payload: string): string | null {
  const after = payload.includes("://") ? payload.split("://")[1] : payload;
  const slash = after.indexOf("/");
  if (slash < 0) return null;
  let path: string;
  try {
    path = decodeURIComponent(after.slice(slash));
  } catch {
    path = after.slice(slash);
  }
  const drive = path.match(/^\/([A-Za-z]:)(\/.*)?$/);
  if (drive) {
    const rest = (drive[2] ?? "").replaceAll("/", "\\");
    return `${drive[1]}${rest}`;
  }
  return path;
}
