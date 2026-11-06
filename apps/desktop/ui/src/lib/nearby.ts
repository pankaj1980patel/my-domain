// Nearby media mode (mdcore `features::nearby`): how a running stream
// with a device stands, worded for the badge and the details.

import type { NearbyStatus_Serialize as NearbyStatus } from "../bindings";
import { t } from "../i18n";

export type { NearbyStatus };

/** The "Nearby · low latency" badge shows: a stream runs, on a local link. */
export function showBadge(s: NearbyStatus | null | undefined): boolean {
  return !!s && s.media && s.low_latency;
}

/** "12 ms" or "12 ms (was 140 ms)"; "" when not measured yet. */
export function latencyText(s: NearbyStatus | null | undefined): string {
  if (!s || s.latency_ms == null) return "";
  if (s.before_ms != null && s.before_ms !== s.latency_ms) {
    return t("nearby.latencyWas", { ms: s.latency_ms, was: s.before_ms });
  }
  return t("nearby.latency", { ms: s.latency_ms });
}
