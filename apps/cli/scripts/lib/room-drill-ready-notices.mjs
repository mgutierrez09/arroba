const aggregateReady = /^Room screen: ready · tab Room pointer drill — /
const healthReady = /^Room health: ready$/
const environmentReady = /^Room environment: ready$/
const roomTabReady = /^Room tab: Room pointer drill — /
const actorsPresent = /^Room actors: .+\(present\).*Local user \(present\)$/

export function hasRoomReadyProjection(notices) {
  const readiness = {
    health: false,
    environment: false,
    tab: false,
    actors: false,
  }
  for (const notice of notices) {
    if (notice.startsWith("Room screen:")) {
      const ready = aggregateReady.test(notice)
      readiness.health = ready
      readiness.environment = ready
      readiness.tab = ready
      readiness.actors = ready
    } else if (notice.startsWith("Room health:")) {
      readiness.health = healthReady.test(notice)
    } else if (notice.startsWith("Room environment:")) {
      readiness.environment = environmentReady.test(notice)
    } else if (notice.startsWith("Room tab:")) {
      readiness.tab = roomTabReady.test(notice)
    } else if (notice.startsWith("Room actors:")) {
      readiness.actors = actorsPresent.test(notice)
    }
  }
  return Object.values(readiness).every(Boolean)
}
