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
  return path.replace(/^\/home\/[^/]+/, "~").replace(/^\/root\b/, "~");
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
  try {
    return decodeURIComponent(after.slice(slash));
  } catch {
    return after.slice(slash);
  }
}
