const aggregateReady = /^Room screen: ready · tab Room pointer drill — /
const healthReady = /^Room health: ready$/
const environmentReady = /^Room environment: ready$/
const roomTabReady = /^Room tab: Room pointer drill — /
const actorsPresent = /^Room actors: .+\(present\).*Local user \(present\)$/

export function hasRoomReadyProjection(notices) {
  if (notices.some((notice) => aggregateReady.test(notice))) return true
  return [healthReady, environmentReady, roomTabReady, actorsPresent]
    .every((pattern) => notices.some((notice) => pattern.test(notice)))
}
