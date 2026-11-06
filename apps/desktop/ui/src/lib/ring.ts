// The "find my device" alert sound: a two-tone chime, repeated, made with
// Web Audio so no sound file ships. Stops by itself after RING_MS or when
// stopRing() is called (the ring banner's Stop button, or any click).

const RING_MS = 10_000;

let ctx: AudioContext | undefined;
let timer: ReturnType<typeof setInterval> | undefined;
let stopTimer: ReturnType<typeof setTimeout> | undefined;

/** Webviews only let audio start after the person has interacted with the
 *  page once, so the context is created (and resumed) on the first click or
 *  key press and kept for later rings. */
export function unlockAudio() {
  const unlock = () => {
    ctx ??= new AudioContext();
    void ctx.resume();
  };
  window.addEventListener("pointerdown", unlock, { once: true, capture: true });
  window.addEventListener("keydown", unlock, { once: true, capture: true });
}

function chime(ac: AudioContext) {
  const t0 = ac.currentTime;
  for (const [i, freq] of [988, 1319].entries()) {
    const osc = ac.createOscillator();
    const gain = ac.createGain();
    osc.type = "sine";
    osc.frequency.value = freq;
    const start = t0 + i * 0.18;
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(0.4, start + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.35);
    osc.connect(gain).connect(ac.destination);
    osc.start(start);
    osc.stop(start + 0.4);
  }
}

/** Ring for `ms` (RING_MS by default); `onDone` runs when it stops by
 *  itself. A timer ending here rings longer. */
export function startRing(onDone: () => void, ms = RING_MS) {
  stopRing();
  ctx ??= new AudioContext();
  const ac = ctx;
  void ac.resume();
  chime(ac);
  timer = setInterval(() => chime(ac), 900);
  stopTimer = setTimeout(() => {
    stopRing();
    onDone();
  }, ms);
}

export function stopRing() {
  clearInterval(timer);
  clearTimeout(stopTimer);
  timer = stopTimer = undefined;
}
