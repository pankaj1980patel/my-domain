// Opening Help from anywhere: the app shell registers how (setHelpOpener),
// and a "?" button next to a setting or tile opens its topic.

import { t } from "../i18n";
import { topicById } from "./topics";

let opener: ((topic?: string) => void) | undefined;

/** Called once by the app shell. */
export function setHelpOpener(fn: (topic?: string) => void) {
  opener = fn;
}

/** Show Help, at [topic] when given. */
export function openHelp(topic?: string) {
  opener?.(topic);
}

/** A small "?" that opens Help at [topic]. */
export function HelpLink(props: { topic: string }) {
  const title = () => topicById(props.topic)?.title ?? t("help.title");
  return (
    <button
      type="button"
      class="help-link"
      aria-label={t("help.about", { topic: title() })}
      title={t("help.about", { topic: title() })}
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        openHelp(props.topic);
      }}
    >
      ?
    </button>
  );
}
