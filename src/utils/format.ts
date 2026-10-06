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

export function presenceLine(input: {
  time: string;
  date: string;
  network: string;
  computerName: string;
  serverName: string;
  state: string;
}): string {
  const computer = input.computerName.trim() || "This computer";
  const server = input.serverName.trim() || computer;
  const verb =
    input.state === "ONLINE" || input.state === "DEGRADED"
      ? "is online"
      : input.state === "OFFLINE" || input.state === "ERROR"
        ? "is offline"
        : "is connecting";
  return [input.time, input.date, input.network.trim(), `${computer} ${verb} ${server}`, "Prysel Unit Agents"]
    .filter((part) => part.length > 0)
    .join(" · ");
}

export function formatNow(now: Date): { time: string; date: string } {
  const time = new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(now);
  const date = new Intl.DateTimeFormat(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
    year: "numeric",
  }).format(now);
  return { time, date };
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
