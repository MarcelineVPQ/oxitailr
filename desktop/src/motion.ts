// Whether motion should stop: the system's reduce-motion setting (the
// stylesheet follows it with prefers-reduced-motion). For motion started in
// script, which the stylesheet can't stop (a deep link's scroll and flash).
const REDUCED_QUERY = "(prefers-reduced-motion: reduce)";

export function reducedMotionNow(): boolean {
  return typeof matchMedia === "function" && matchMedia(REDUCED_QUERY).matches;
}
