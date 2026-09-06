import { createHmac } from "node:crypto"
import { roomDrillCompanionTimeoutMs } from "./room-drill-companion-budget.mjs"

export function roomDrillRelayToken({ issuer, secret, machineId, subject, subjectKind,
  actions, userId = null, env = process.env, nowMs = Date.now() }) {
  // Static fixture credentials have no hosted renewal service. Cover the
  // bounded companion wait plus provisioning and cleanup, not just setup.
  // Browser-issued credentials retain their normal expiry/renewal behavior.
  const companionMs = env.CHARIOX_ROOM_DRILL_COORDINATION_DIR?.trim()
    ? roomDrillCompanionTimeoutMs(env) : 0
  const claims = {
    issuer,
    subject,
    subject_kind: subjectKind,
    realm_id: "local-dev",
    allowed_actions: actions,
    allowed_targets: null,
    issued_at_ms: nowMs,
    expires_at_ms: nowMs + 15 * 60_000 + companionMs,
    token_id: `${subject}-${nowMs}`,
    account_id: "local-dev-account",
    organization_id: null,
    user_id: userId,
    device_id: null,
    machine_id: subjectKind === "kernel" ? machineId : null,
    client_id: subjectKind === "client" ? subject : null,
    session_id: null,
    public_key_thumbprint: null,
    entitlements_version: "room-pointer-drill",
  }
  const payload = Buffer.from(JSON.stringify(claims)).toString("base64url")
  const signature = createHmac("sha256", secret).update(payload).digest("base64url")
  return `chariox-scoped-v1.${payload}.${signature}`
}
