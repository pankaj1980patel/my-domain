// Whether a device said it may be woken (Wake-on-LAN, `wol.rs`), as a
// signal that follows the device and is asked again when its status
// changes (that's where it's learnt). Used to offer "Wake it from sleep"
// in schedules and automations.

import { createEffect, createSignal, on, type Accessor } from "solid-js";
import { commands } from "../bindings";
import { state } from "./core";

export function createCanWake(node: Accessor<string>): Accessor<boolean> {
  const [can, setCan] = createSignal(false);
  createEffect(
    on(
      () => [node(), state.status[node()]] as const,
      ([n]) => {
        if (!n) return setCan(false);
        commands.canWake(n).then(setCan, () => setCan(false));
      },
    ),
  );
  return can;
}
