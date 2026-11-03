// The brand mark as the kit shows it: the galaxy logo, the "My-Domain"
// wordmark with "Domain" in the accent colour, and the gold tagline.

import { theme } from "../lib/theme";
import { t } from "../i18n";

/** The galaxy logo for the current theme: the dark mark everywhere except
 *  Cosmic Light, which uses the light galaxy (no frame of its own). */
export function logoSrc(): string {
  return theme() === "light" ? "/logo-light.png" : "/logo.png";
}

export function Wordmark(props: { class?: string }) {
  return (
    <span class={`wordmark ${props.class ?? ""}`}>
      {t("brand.wordmarkLead")}
      <span class="wordmark-accent">{t("brand.wordmarkAccent")}</span>
    </span>
  );
}

export function Tagline() {
  return <span class="brand-tagline">{t("brand.tagline")}</span>;
}
