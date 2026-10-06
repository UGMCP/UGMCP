import { useEffect, useState } from "react";
import { api } from "../services/api";
import type { HostIdentity } from "../types";
import { formatNow, presenceLine } from "./format";

export function usePresence(state: string): string {
  const [now, setNow] = useState(() => new Date());
  const [host, setHost] = useState<HostIdentity | null>(null);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    let stop = false;
    const load = () => {
      void api
        .hostIdentity()
        .then((value) => {
          if (!stop) setHost(value);
        })
        .catch(() => undefined);
    };
    load();
    const timer = window.setInterval(load, 30000);
    return () => {
      stop = true;
      window.clearInterval(timer);
    };
  }, []);

  const clock = formatNow(now);
  return presenceLine({
    time: clock.time,
    date: clock.date,
    network: host?.network ?? "",
    computerName: host?.computerName ?? "This computer",
    serverName: host?.serverName ?? "",
    state,
  });
}
