// Notes, Timers and Read later: one place, three tabs. Each screen draws
// them under its own header, like every other page's tabs.

import { t } from "../i18n";
import { timers } from "../lib/timers";
import { unreadCount } from "../lib/readLater";
import { Tabs } from "./Tabs";

export type Tool = "notes" | "timers" | "readLater";

export function ToolTabs(props: { selected: Tool; onSelect?: (tool: Tool) => void }) {
  // Running timers (ended ones too, until stopped).
  const activeTimers = () => Object.values(timers.byId).filter((x) => x.state === "running").length;
  return (
    <Tabs
      label={t("nav.notesGroup")}
      idBase="tools"
      tabs={[
        { id: "notes", label: t("nav.notes"), icon: "notes" },
        { id: "timers", label: t("nav.timers"), icon: "timers", count: activeTimers() },
        { id: "readLater", label: t("nav.readLater"), icon: "readLater", count: unreadCount() },
      ]}
      selected={props.selected}
      onSelect={(id) => props.onSelect?.(id as Tool)}
    />
  );
}
