import { describe, expect, it } from "vitest";
import { osc7Path, shortPath } from "./format";
import { gridColumns, resolvePhase } from "./startup";

describe("resolvePhase", () => {
  it("opens the desktop for a valid session and for offline work", () => {
    expect(resolvePhase({ fatal: false, session: { authenticated: true, expired: false }, offlineChosen: false })).toBe("desktop");
    expect(resolvePhase({ fatal: false, session: { authenticated: false, expired: false }, offlineChosen: true })).toBe("desktop");
  });

  it("keeps login, expiry, and fatal states distinct", () => {
    expect(resolvePhase({ fatal: false, session: { authenticated: false, expired: false }, offlineChosen: false })).toBe("login");
    expect(resolvePhase({ fatal: false, session: { authenticated: false, expired: true }, offlineChosen: false })).toBe("expired");
    expect(resolvePhase({ fatal: true, session: { authenticated: true, expired: false }, offlineChosen: false })).toBe("fatal");
  });
});

describe("paths", () => {
  it("reads OSC 7 locations on unix and windows", () => {
    expect(osc7Path("file://host/home/ubuntu")).toBe("/home/ubuntu");
    expect(osc7Path("file://localhost/C:/Users/ada")).toBe("C:\\Users\\ada");
    expect(shortPath("/home/ubuntu/work")).toBe("~/work");
    expect(shortPath("/Users/ada/work")).toBe("~/work");
    expect(shortPath("C:\\Users\\ada\\work")).toBe("~\\work");
  });
});

describe("gridColumns", () => {
  it("uses one, two, or three columns for common desktop widths", () => {
    expect(gridColumns(800)).toBe(1);
    expect(gridColumns(1366)).toBe(2);
    expect(gridColumns(1920)).toBe(3);
    expect(gridColumns(2560)).toBe(3);
    expect(gridColumns(3840)).toBe(3);
  });
});
