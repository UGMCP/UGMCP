import type { Phase, SessionView } from "../types";

export function resolvePhase(input: {
  fatal: boolean;
  session: Pick<SessionView, "authenticated" | "expired">;
  offlineChosen: boolean;
}): Phase {
  if (input.fatal) return "fatal";
  if (input.offlineChosen) return "desktop";
  if (input.session.authenticated) return "desktop";
  if (input.session.expired) return "expired";
  return "login";
}

export function gridColumns(width: number): number {
  if (width >= 1500) return 3;
  if (width >= 900) return 2;
  return 1;
}
