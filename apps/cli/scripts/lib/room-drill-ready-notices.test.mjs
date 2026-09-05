import assert from "node:assert/strict"
import test from "node:test"

import { hasRoomReadyProjection } from "./room-drill-ready-notices.mjs"

test("accepts the aggregate Room ready projection", () => {
  assert.equal(hasRoomReadyProjection([
    "Room screen: ready · tab Room pointer drill — http://host.docker.internal:1234/click · actors Agent, Local user · input available",
  ]), true)
})

test("accepts the equivalent event-by-event Room ready projection", () => {
  assert.equal(hasRoomReadyProjection([
    "Room health: ready",
    "Room tab: Room pointer drill — http://host.docker.internal:1234/click",
    "Room environment: ready",
    "Room actors: Agent (present), Local user (present)",
  ]), true)
})

test("rejects an incomplete or unhealthy projection", () => {
  assert.equal(hasRoomReadyProjection([
    "Room tab: Room pointer drill — http://host.docker.internal:1234/click",
    "Room environment: ready",
    "Room actors: Agent (present), Local user (present)",
  ]), false)
})

test("rejects readiness superseded by a newer unhealthy projection", () => {
  assert.equal(hasRoomReadyProjection([
    "Room health: ready",
    "Room tab: Room pointer drill — http://host.docker.internal:1234/click",
    "Room environment: ready",
    "Room actors: Agent (present), Local user (present)",
    "Room health: unavailable",
  ]), false)
})
