// Shared by the OSS waiter and the Web launcher. Long deadlines are opt-in;
// allow the plan's 24-hour gate plus bounded setup/final-verification time.
const maximumSoakMs = 24 * 60 * 60 * 1000
const verificationAllowanceMs = 10 * 60 * 1000
const maximumTimeoutMs = maximumSoakMs + verificationAllowanceMs

export function roomDrillCompanionTimeoutMs(env, { activeDurationMs = 0, defaultTimeoutMs = 180_000 } = {}) {
  if (!Number.isSafeInteger(activeDurationMs) || activeDurationMs < 0 || activeDurationMs > maximumSoakMs) {
    throw new Error("Room active soak duration must be an integer from 0 to 86400000ms")
  }
  const minimumTimeoutMs = activeDurationMs > 0 ? activeDurationMs + verificationAllowanceMs : 1
  const configured = env.CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS?.trim()
  const timeoutMs = configured ? Number(configured)
    : activeDurationMs > 0 ? minimumTimeoutMs : defaultTimeoutMs
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > maximumTimeoutMs) {
    throw new Error(`CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS must be an integer from 1 to ${maximumTimeoutMs}`)
  }
  if (timeoutMs < minimumTimeoutMs) {
    throw new Error(`CHARIOX_ROOM_DRILL_COMPANION_TIMEOUT_MS must allow the active soak plus 600000ms for setup and verification; need at least ${minimumTimeoutMs}ms`)
  }
  return timeoutMs
}
