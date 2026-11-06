// Verification codes from your phone (otp_codes.rs): "Copy verification
// codes automatically", off by default.

import { createSignal, onMount } from "solid-js";
import { commands } from "../bindings";
import { t } from "../i18n";
import { HelpLink } from "../help/HelpLink";

export function OtpSetting() {
  const [on, setOn] = createSignal(false);
  onMount(() => void commands.otpAutoCopy().then(setOn, () => {}));
  const toggle = (v: boolean) => {
    setOn(v);
    void commands.setOtpAutoCopy(v);
  };
  return (
    <>
      <label class="switch-row">
        <span>
          {t("otp.autoCopy")}
          <HelpLink topic="notifications" />
        </span>
        <input type="checkbox" role="switch" checked={on()} onChange={(e) => toggle(e.currentTarget.checked)} />
      </label>
      <p class="muted small">{t("otp.autoCopyHint")}</p>
    </>
  );
}
