import { randomUUID } from "node:crypto"
import {
  closeSync,
  constants as fsConstants,
  fstatSync,
  lstatSync,
  openSync,
  readFileSync,
  unlinkSync,
} from "node:fs"

import WebSocket from "ws"

import type { KernelEvent } from "./kernel-events.js"
import type {
  IpcEnvelope,
  KernelSocketLane,
  KernelTransportEventFrame,
  KernelTransportResponseFrame,
  RelayCloseFrame,
  RelayConnectedFrame,
  RelayEventFrame,
  RelayResponseFrame,
  RelayTarget,
} from "./kernel-transport-frames.js"
import { normalizeWebSocketRequest } from "./kernel-transport-requests.js"
import {
  buildKernelSubscriptionTransportRequest,
  createKernelSessionSubscriptionStart,
  createWaitingRoomInventorySubscriptionStart,
  kernelSubscriptionScopeValue,
  type KernelSubscriptionState,
} from "./kernel-subscriptions.js"
import { LocalIpcError } from "./local-ipc-error.js"
import { sendLocalSocketRequest } from "./local-socket-transport.js"
import { createRelayKeypair, decryptRelayPayload } from "./relay-crypto.js"
import {
  buildRelayConnectFrame,
  buildRelaySubscribeFrame,
  buildRelayUnsubscribeFrame,
  normalizeRelayRequest,
} from "./relay-transport.js"
import { KernelPendingRequestRegistry } from "./websocket-pending-requests.js"
import { KernelRequestLifetime, waitForKernelRequestReplay } from "./websocket-request-lifetime.js"
import { formatTransportError, isWebSocketEndpoint } from "./websocket-transport-diagnostics.js"

// Slice start can cold-build the managed Linux image before returning the
// worker kernel endpoint. Keep the control request open long enough for first
// run provisioning while lifecycle progress remains request/response based.
const IPC_TIMEOUT_MS = 600_000
const DEFAULT_KERNEL_EVENT_STALE_MS = 0
const DEFAULT_KERNEL_PING_INTERVAL_MS = 5_000
const DEFAULT_KERNEL_MAX_MISSED_PONGS = 2
const IPC_WEBSOCKET_CLOSE_TIMEOUT_MS = 1_000
const IPC_CLIENT_CLOSE_TIMEOUT_MS = 1_500
const KERNEL_RECONNECT_BASE_DELAY_MS = 250
const KERNEL_RECONNECT_MAX_DELAY_MS = 5_000
const KERNEL_RECONNECT_JITTER_MS = 250
const KERNEL_CONTROL_REQUEST_RETRY_DEADLINE_MS = 60_000
const KERNEL_CONTROL_RESPONSE_STALL_MS = 5_000
const MAX_KERNEL_LOCAL_AUTH_TOKEN_BYTES = 8 * 1024

export type { KernelEvent } from "./kernel-events.js"
export { LocalIpcError } from "./local-ipc-error.js"

type BoundKernelLocalAuthCredential = {
  endpoint: string
  token: string
}

const hostedPublicationEnvironmentNames = [
  "CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL",
  "CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE",
  "CHARIOX_PUBLICATION_CLOUD_API_URL",
  "CHARIOX_PUBLICATION_CLOUD_DEPLOYMENT_ID",
  "CHARIOX_PUBLICATION_CLOUD_RUNNER_KEY",
] as const

let kernelLocalAuthCredentialFromEnvironment: BoundKernelLocalAuthCredential | undefined

export function consumeKernelLocalAuthTokenFromEnv(endpoint = configuredLocalKernelEndpoint()): string | undefined {
  const rawEnvironmentToken = process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN
  const rawTokenFile = process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
  delete process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN
  delete process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
  if (rawEnvironmentToken !== undefined && rawTokenFile !== undefined) {
    throw new Error("kernel local auth token and token file cannot both be configured")
  }
  if (kernelLocalAuthCredentialFromEnvironment) {
    if (rawEnvironmentToken !== undefined || rawTokenFile !== undefined) {
      throw new Error("kernel local auth credential cannot be reconfigured after consumption")
    }
    const canonicalEndpoint = requireCanonicalLoopbackKernelEndpoint(endpoint)
    if (canonicalEndpoint !== kernelLocalAuthCredentialFromEnvironment.endpoint) {
      throw new Error(
        `kernel local auth credential is bound to kernel endpoint ${kernelLocalAuthCredentialFromEnvironment.endpoint}`,
      )
    }
    return kernelLocalAuthCredentialFromEnvironment.token
  }
  if (rawEnvironmentToken === undefined && rawTokenFile === undefined) return undefined

  const canonicalEndpoint = requireCanonicalLoopbackKernelEndpoint(endpoint)
  const environmentToken = rawEnvironmentToken?.trim()
  const tokenFile = rawTokenFile?.trim()
  if (rawEnvironmentToken !== undefined && !environmentToken) {
    throw new Error("kernel local auth token must not be empty")
  }
  if (rawTokenFile !== undefined && !tokenFile) {
    throw new Error("kernel local auth token file path must not be empty")
  }
  if (environmentToken && isHostedPublicationGateway()) {
    throw new Error("hosted publication gateways require a one-shot kernel local auth token file")
  }
  const token = environmentToken ?? readPrivateKernelLocalAuthToken(tokenFile!)
  kernelLocalAuthCredentialFromEnvironment = { endpoint: canonicalEndpoint, token }
  return token
}

function readPrivateKernelLocalAuthToken(path: string): string {
  let descriptor: number
  try {
    descriptor = openSync(path, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW)
  } catch (error) {
    throw new Error(`kernel local auth token file could not be opened safely: ${String(error)}`)
  }
  try {
    const metadata = fstatSync(descriptor)
    const currentUid = process.getuid?.()
    if (
      !metadata.isFile()
      || (metadata.mode & 0o077) !== 0
      || (currentUid !== undefined && metadata.uid !== currentUid)
      || metadata.nlink !== 1
      || metadata.size > MAX_KERNEL_LOCAL_AUTH_TOKEN_BYTES
    ) {
      throw new Error(
        "kernel local auth token file must be a bounded, single-link owned regular file with mode 0600",
      )
    }
    const pathMetadata = lstatSync(path)
    if (
      !pathMetadata.isFile()
      || pathMetadata.isSymbolicLink()
      || pathMetadata.dev !== metadata.dev
      || pathMetadata.ino !== metadata.ino
    ) {
      throw new Error("kernel local auth token file changed while it was being consumed")
    }
    unlinkSync(path)
    if (fstatSync(descriptor).nlink !== 0) {
      throw new Error("kernel local auth token file was not consumed from its validated descriptor")
    }
    const token = readFileSync(descriptor, "utf8").trim()
    if (!token) throw new Error("kernel local auth token file must not be empty")
    return token
  } finally {
    closeSync(descriptor)
  }
}

function requireCanonicalLoopbackKernelEndpoint(endpoint: string): string {
  const canonicalEndpoint = canonicalLoopbackKernelEndpoint(endpoint)
  if (!canonicalEndpoint) {
    throw new Error("kernel local auth credentials require an exact canonical loopback kernel endpoint")
  }
  return canonicalEndpoint
}

function canonicalLoopbackKernelEndpoint(endpoint: string): string | null {
  let url: URL
  try {
    url = new URL(endpoint)
  } catch {
    return null
  }
  if (
    url.protocol !== "ws:"
    || (url.hostname !== "127.0.0.1" && url.hostname !== "[::1]")
    || url.username !== ""
    || url.password !== ""
    || url.pathname !== "/"
    || url.search !== ""
    || url.hash !== ""
  ) {
    return null
  }
  return url.href
}

function configuredLocalKernelEndpoint() {
  return process.env.CHARIOX_KERNEL_URL?.trim()
    || `ws://${process.env.CHARIOX_KERNEL_HOST?.trim() || "127.0.0.1"}:${process.env.CHARIOX_KERNEL_PORT?.trim() || "43118"}`
}

function isHostedPublicationGateway() {
  return hostedPublicationEnvironmentNames.some((name) => Boolean(process.env[name]?.trim()))
}

type LocalIpcClientOptions = {
  localAuthToken?: string | undefined
  relayAuthToken?: string | undefined
  targetDaemonId?: string | undefined
  targetDaemonAlias?: string | undefined
  kernelEventStaleMs?: number | undefined
  kernelPingIntervalMs?: number | undefined
  kernelMaxMissedPongs?: number | undefined
  reconnectJitterMs?: number | undefined
  reconnectRandom?: (() => number) | undefined
  controlRequestRetryDeadlineMs?: number | undefined
  controlResponseStallMs?: number | undefined
}

export class LocalIpcClient {
  readonly socketPath: string
  private readonly localAuthEndpoint: string | null
  private readonly localAuthToken: string | null
  private readonly relayAuthToken: string | null
  private readonly relayTarget: RelayTarget | null
  private controlWebsocket: WebSocket | null = null
  private eventWebsocket: WebSocket | null = null
  private connectingControlWebsocket: WebSocket | null = null
  private connectingEventWebsocket: WebSocket | null = null
  private controlWebsocketConnectPromise: Promise<WebSocket> | null = null
  private eventWebsocketConnectPromise: Promise<WebSocket> | null = null
  private readonly pendingRequests = new KernelPendingRequestRegistry(IPC_TIMEOUT_MS)
  private readonly requestLifetime = new KernelRequestLifetime()
  private eventHandlers = new Set<(event: KernelEvent) => void>()
  private activeKernelSubscription: KernelSubscriptionState | null = null
  private reconnectTimeout: NodeJS.Timeout | null = null
  private reconnectDelayMs = 250
  private lastReceivedEventId: number | null = null
  private lastKernelEventAtMs = 0
  private kernelEventWatchdog: NodeJS.Timeout | null = null
  private controlHeartbeat: NodeJS.Timeout | null = null
  private eventHeartbeat: NodeJS.Timeout | null = null
  private readonly reconnectJitterMs: number
  private readonly reconnectRandom: () => number
  private readonly controlRequestRetryDeadlineMs: number
  private readonly controlResponseStallMs: number
  private missedControlPongs = 0
  private missedEventPongs = 0
  private suppressNextControlCloseEvent = false
  private suppressNextEventCloseEvent = false
  private controlRelayDaemonPublicKey: string | null = null
  private eventRelayDaemonPublicKey: string | null = null
  private readonly kernelEventStaleMs: number
  private readonly kernelPingIntervalMs: number
  private readonly kernelMaxMissedPongs: number

  constructor(endpoint: string, options: LocalIpcClientOptions = {}) {
    this.socketPath = endpoint
    const staleMs = options.kernelEventStaleMs ?? DEFAULT_KERNEL_EVENT_STALE_MS
    this.kernelEventStaleMs = staleMs > 0 ? Math.max(staleMs, 250) : 0
    this.kernelPingIntervalMs = Math.max(options.kernelPingIntervalMs ?? DEFAULT_KERNEL_PING_INTERVAL_MS, 250)
    this.kernelMaxMissedPongs = Math.max(options.kernelMaxMissedPongs ?? DEFAULT_KERNEL_MAX_MISSED_PONGS, 1)
    this.reconnectJitterMs = Math.max(options.reconnectJitterMs ?? KERNEL_RECONNECT_JITTER_MS, 0)
    this.reconnectRandom = options.reconnectRandom ?? Math.random
    this.controlRequestRetryDeadlineMs = Math.max(
      options.controlRequestRetryDeadlineMs ?? KERNEL_CONTROL_REQUEST_RETRY_DEADLINE_MS,
      0,
    )
    this.controlResponseStallMs = Math.max(
      options.controlResponseStallMs ?? KERNEL_CONTROL_RESPONSE_STALL_MS,
      10,
    )
    this.relayAuthToken = options.relayAuthToken?.trim() || null
    const explicitLocalAuthToken = options.localAuthToken?.trim()
    if (options.localAuthToken !== undefined && !explicitLocalAuthToken) {
      throw new Error("kernel local auth token must not be empty")
    }
    if (explicitLocalAuthToken && (
      process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN !== undefined
      || process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE !== undefined
    )) {
      throw new Error("explicit and environment kernel local auth credentials cannot both be configured")
    }
    if (this.relayAuthToken && (
      explicitLocalAuthToken
      || process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN !== undefined
      || process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE !== undefined
    )) {
      throw new Error("kernel local auth credentials cannot be used with relay transport")
    }
    if (explicitLocalAuthToken && isHostedPublicationGateway()) {
      throw new Error("hosted publication gateways require a one-shot kernel local auth token file")
    }
    this.localAuthToken = this.relayAuthToken
      ? null
      : explicitLocalAuthToken ?? consumeKernelLocalAuthTokenFromEnv(endpoint) ?? null
    this.localAuthEndpoint = this.localAuthToken
      ? requireCanonicalLoopbackKernelEndpoint(endpoint)
      : null
    this.relayTarget = this.relayAuthToken
      ? {
        daemon_id: options.targetDaemonId?.trim() || null,
        daemon_alias: options.targetDaemonAlias?.trim() || null,
      }
      : null
  }

  supportsKernelEvents() {
    return isWebSocketEndpoint(this.socketPath)
  }

  private isRelayMode() {
    return this.relayAuthToken != null
  }

  send<TResponse>(request: unknown): Promise<TResponse> {
    if (isWebSocketEndpoint(this.socketPath)) {
      return this.sendWebSocket(request)
    }
    return this.sendLocalSocket(request)
  }

  async subscribeToKernelEvents(sessionId: string, attachmentId: string): Promise<void> {
    if (!this.supportsKernelEvents()) {
      return
    }
    const start = createKernelSessionSubscriptionStart({
      previous: this.activeKernelSubscription,
      lastReceivedEventId: this.lastReceivedEventId,
      sessionId,
      attachmentId,
      relaySubscriptionId: this.isRelayMode() ? randomUUID() : null,
    })
    if (start.resetLastReceivedEventId) {
      this.lastReceivedEventId = null
    }
    this.activeKernelSubscription = start.subscription
    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(sessionId, attachmentId, start.resumeFromEventId)
      } else {
        await this.sendWebSocket<Record<string, unknown>>(
          buildKernelSubscriptionTransportRequest(start.subscription, start.resumeFromEventId),
          "event",
        )
      }
      this.clearReconnectState()
      this.markKernelEventReceived()
    } catch (error) {
      this.scheduleReconnect()
      throw error
    }
  }

  async subscribeToWaitingRoomInventory(): Promise<void> {
    if (!this.supportsKernelEvents()) {
      return
    }
    const start = createWaitingRoomInventorySubscriptionStart({
      previous: this.activeKernelSubscription,
      lastReceivedEventId: this.lastReceivedEventId,
      relaySubscriptionId: this.isRelayMode() ? randomUUID() : null,
    })
    if (start.resetLastReceivedEventId) {
      this.lastReceivedEventId = null
    }
    this.activeKernelSubscription = start.subscription
    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(
          start.subscription.sessionId,
          start.subscription.attachmentId,
          start.resumeFromEventId,
          kernelSubscriptionScopeValue(start.subscription),
        )
      } else {
        await this.sendWebSocket<Record<string, unknown>>(
          buildKernelSubscriptionTransportRequest(start.subscription, start.resumeFromEventId),
          "event",
        )
      }
      this.clearReconnectState()
      this.markKernelEventReceived()
    } catch (error) {
      this.scheduleReconnect()
      throw error
    }
  }

  async unsubscribeFromKernelEvents(): Promise<void> {
    if (!this.supportsKernelEvents()) {
      return
    }
    const subscription = this.activeKernelSubscription
    this.activeKernelSubscription = null
    this.clearReconnectState()
    this.clearKernelEventWatchdog()
    const socket = this.getWebSocket("event")
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return
    }
    if (this.isRelayMode()) {
      if (!subscription?.relaySubscriptionId || !subscription.relayPrivateKey) {
        return
      }
      await this.sendRelayUnsubscribe(subscription.relaySubscriptionId, subscription.relayPrivateKey)
    } else {
      await this.sendWebSocket<Record<string, unknown>>({
        __kernel_transport: {
          type: "unsubscribe",
        },
      }, "event")
    }
  }

  async restartKernelEventStream(): Promise<void> {
    if (!this.supportsKernelEvents() || !this.activeKernelSubscription) {
      return
    }
    this.clearReconnectState()
    this.clearKernelEventWatchdog()
    this.clearKernelHeartbeat("event")
    const socket = this.getWebSocket("event")
    if (socket && socket.readyState !== WebSocket.CLOSED) {
      this.suppressNextEventCloseEvent = true
      socket.terminate()
      this.setWebSocket("event", null)
      this.setWebSocketConnectPromise("event", null)
    }
    this.scheduleReconnect(25)
  }

  onKernelEvent(handler: (event: KernelEvent) => void) {
    this.eventHandlers.add(handler)
    return () => {
      this.eventHandlers.delete(handler)
    }
  }

  async close(): Promise<void> {
    this.clearRuntimeTransportState("kernel client closed")
    let timedOut = false
    let timeout: ReturnType<typeof setTimeout> | undefined
    await Promise.race([
      Promise.all([
        this.closeWebSocket("control"),
        this.closeWebSocket("event"),
      ]).then(() => undefined),
      new Promise<void>((resolve) => {
        timeout = setTimeout(() => {
          timedOut = true
          resolve()
        }, IPC_CLIENT_CLOSE_TIMEOUT_MS)
      }),
    ])
    if (timeout) {
      clearTimeout(timeout)
    }
    if (timedOut) {
      this.destroy()
    }
  }

  destroy(): void {
    this.clearRuntimeTransportState("kernel client destroyed")
    this.destroyWebSocket("control")
    this.destroyWebSocket("event")
  }

  private clearRuntimeTransportState(pendingMessage: string): void {
    this.requestLifetime.retire(pendingMessage)
    this.activeKernelSubscription = null
    this.clearReconnectState()
    this.clearKernelEventWatchdog()
    this.clearKernelHeartbeat("control")
    this.clearKernelHeartbeat("event")
    this.controlRelayDaemonPublicKey = null
    this.eventRelayDaemonPublicKey = null
    this.rejectPending(pendingMessage)
  }

  private sendLocalSocket<TResponse>(request: unknown): Promise<TResponse> {
    return sendLocalSocketRequest(this.socketPath, request, IPC_TIMEOUT_MS)
  }

  private async sendWebSocket<TResponse>(request: unknown, lane: KernelSocketLane = "control"): Promise<TResponse> {
    const lifetime = this.requestLifetime.capture()
    const requestId = randomUUID()
    const retryUntilMs = lane === "control"
      ? Date.now() + this.controlRequestRetryDeadlineMs
      : Date.now()
    let retryDelayMs = KERNEL_RECONNECT_BASE_DELAY_MS

    for (;;) {
      lifetime.throwIfAborted()
      let socket: WebSocket
      try {
        socket = await this.ensureWebSocket(lane)
      } catch (error) {
        lifetime.throwIfAborted()
        if (!this.shouldReplayWebSocketRequest(error, lane, retryUntilMs)) {
          throw error
        }
        this.destroyWebSocket(lane)
        retryDelayMs = await this.waitBeforeWebSocketRequestReplay(retryDelayMs, retryUntilMs, lifetime)
        continue
      }

      lifetime.throwIfAborted()
      const pending = this.pendingRequests.register<TResponse>(
        requestId,
        lane,
        this.requestAttemptTimeoutMs(lane, retryUntilMs),
      )

      try {
        const relayRequest = this.isRelayMode()
          ? normalizeRelayRequest(requestId, request, this.relayTarget, this.getRelayDaemonPublicKey(lane))
          : null
        if (relayRequest) {
          pending.setRelayPrivateKey(relayRequest.privateKey)
        }
        const payload = relayRequest
          ? relayRequest.frame
          : normalizeWebSocketRequest(requestId, request)
        socket.send(JSON.stringify(payload))
      } catch (error) {
        pending.reject(new LocalIpcError("write kernel request", error instanceof Error ? error.message : String(error), "write_failed", true))
      }

      try {
        return await pending.promise
      } catch (error) {
        lifetime.throwIfAborted()
        if (!this.shouldReplayWebSocketRequest(error, lane, retryUntilMs)) {
          throw error
        }
        this.destroyWebSocket(lane)
        retryDelayMs = await this.waitBeforeWebSocketRequestReplay(retryDelayMs, retryUntilMs, lifetime)
      }
    }
  }

  private shouldReplayWebSocketRequest(error: unknown, lane: KernelSocketLane, retryUntilMs: number): boolean {
    return lane === "control"
      && Date.now() < retryUntilMs
      && error instanceof LocalIpcError
      && error.retryable
      && (error.code === "connection_closed" || error.code === "write_failed" || error.code === "request_timeout")
  }

  private requestAttemptTimeoutMs(lane: KernelSocketLane, retryUntilMs: number): number {
    if (lane !== "control") {
      return IPC_TIMEOUT_MS
    }
    const remainingRetryMs = retryUntilMs - Date.now()
    if (remainingRetryMs <= this.controlResponseStallMs) {
      return IPC_TIMEOUT_MS
    }
    return this.controlResponseStallMs
  }

  private async waitBeforeWebSocketRequestReplay(delayMs: number, retryUntilMs: number, lifetime: AbortSignal): Promise<number> {
    lifetime.throwIfAborted()
    const remainingMs = retryUntilMs - Date.now()
    if (remainingMs <= 0) {
      return delayMs
    }
    const waitMs = Math.min(this.reconnectDelayWithJitter(delayMs), remainingMs)
    if (waitMs > 0) {
      await waitForKernelRequestReplay(waitMs, lifetime)
    }
    return this.nextReconnectDelayMs(delayMs)
  }

  private async sendRelaySubscribe(
    sessionId: string,
    attachmentId: string,
    resumeFromEventId: number | null,
    subscriptionScope?: string,
  ): Promise<void> {
    const lane: KernelSocketLane = "event"
    const socket = await this.ensureWebSocket(lane)
    const requestId = randomUUID()
    const subscription = this.activeKernelSubscription
    if (!subscription?.relaySubscriptionId) {
      throw new LocalIpcError("write relay subscribe", "relay subscription state is missing")
    }
    const subscriptionId = subscription.relaySubscriptionId
    const keypair = createRelayKeypair()
    subscription.relayPrivateKey = keypair.privateKey

    const pending = this.pendingRequests.register<void>(requestId, lane)
    pending.setRelayPrivateKey(keypair.privateKey)

    try {
      const frame = buildRelaySubscribeFrame({
        requestId,
        subscriptionId,
        target: this.relayTarget,
        sessionId,
        attachmentId,
        clientPublicKey: keypair.publicKeyBase64,
        resumeFromEventId,
        subscriptionScope,
      })
      socket.send(JSON.stringify(frame))
    } catch (error) {
      pending.reject(new LocalIpcError("write relay subscribe", error instanceof Error ? error.message : String(error), "write_failed", true))
    }

    await pending.promise
  }

  private async sendRelayUnsubscribe(subscriptionId: string, privateKey: Buffer): Promise<void> {
    const lane: KernelSocketLane = "event"
    const socket = await this.ensureWebSocket(lane)
    const requestId = randomUUID()

    const pending = this.pendingRequests.register<void>(requestId, lane)
    pending.setRelayPrivateKey(privateKey)

    try {
      const frame = buildRelayUnsubscribeFrame(requestId, subscriptionId, privateKey)
      socket.send(JSON.stringify(frame))
    } catch (error) {
      pending.reject(new LocalIpcError("write relay unsubscribe", error instanceof Error ? error.message : String(error), "write_failed", true))
    }

    await pending.promise
  }

  private async ensureWebSocket(lane: KernelSocketLane = "control"): Promise<WebSocket> {
    const existing = this.getWebSocket(lane)
    if (existing?.readyState === WebSocket.OPEN) {
      return existing
    }
    const connectPromise = this.getWebSocketConnectPromise(lane)
    if (connectPromise) {
      return connectPromise
    }

    const nextConnectPromise = new Promise<WebSocket>((resolve, reject) => {
      const socket = this.localAuthToken && this.localAuthEndpoint && !this.isRelayMode()
        ? new WebSocket(this.localAuthEndpoint, {
            headers: { authorization: `Bearer ${this.localAuthToken}` },
          })
        : new WebSocket(this.socketPath)
      let settled = false
      this.setConnectingWebSocket(lane, socket)

      const fail = (operation: string, error: unknown, code: string | null = null, retryable = false) => {
        if (settled) {
          return
        }
        settled = true
        if (this.getConnectingWebSocket(lane) === socket) {
          this.setConnectingWebSocket(lane, null)
        }
        this.setWebSocketConnectPromise(lane, null)
        reject(new LocalIpcError(operation, formatTransportError(error, this.socketPath), code, retryable))
      }

      const handleConnectError = (error: unknown) => {
        const authenticationFailed = /Unexpected server response: (?:401|403)/i.test(String(error))
        fail(
          "connect kernel websocket",
          error,
          authenticationFailed ? "authentication_failed" : "connection_closed",
          !authenticationFailed,
        )
      }
      const handleConnectClose = (code: number, reason: Buffer) => {
        const closeMessage = reason.length > 0
          ? reason.toString("utf8")
          : `kernel websocket closed before opening${code ? ` (${code})` : ""}`
        fail("connect kernel websocket", closeMessage, "connection_closed", true)
      }
      const clearConnectListeners = () => {
        socket.off("error", handleConnectError)
        socket.off("close", handleConnectClose)
      }

      socket.once("open", () => {
        const finalizeOpen = () => {
          settled = true
          clearConnectListeners()
          if (this.getConnectingWebSocket(lane) === socket) {
            this.setConnectingWebSocket(lane, null)
          }
          this.setWebSocket(lane, socket)
          this.setWebSocketConnectPromise(lane, null)
          this.setSuppressNextCloseEvent(lane, false)
          this.startKernelHeartbeat(socket, lane)
          socket.on("message", (data: WebSocket.RawData) => {
            this.handleWebSocketMessage(data, lane)
          })
          socket.on("pong", () => {
            this.setMissedKernelPongs(lane, 0)
          })
          socket.once("close", (code: number, reason: Buffer) => {
            if (this.getWebSocket(lane) !== socket) {
              return
            }
            const suppressed = this.getSuppressNextCloseEvent(lane)
            this.setSuppressNextCloseEvent(lane, false)
            const closeMessage = reason.length > 0
              ? reason.toString("utf8")
              : `kernel websocket closed${code ? ` (${code})` : ""}`
            this.rejectPending(closeMessage, lane)
            this.setWebSocket(lane, null)
            this.setRelayDaemonPublicKey(lane, null)
            this.clearKernelHeartbeat(lane)
            if (!suppressed) {
              this.emitSyntheticEvent({
                event: "transport_closed",
                message: closeMessage,
              })
              if (lane === "event") {
                this.scheduleReconnect()
              }
            }
          })
          socket.on("error", (error: unknown) => {
            if (this.getWebSocket(lane) !== socket) {
              return
            }
            const message = formatTransportError(error, this.socketPath)
            const suppressed = this.getSuppressNextCloseEvent(lane)
            this.setSuppressNextCloseEvent(lane, false)
            this.rejectPending(message, lane)
            this.setWebSocket(lane, null)
            this.setRelayDaemonPublicKey(lane, null)
            this.clearKernelHeartbeat(lane)
            if (!suppressed) {
              this.emitSyntheticEvent({
                event: "transport_closed",
                message,
              })
              if (lane === "event") {
                this.scheduleReconnect()
              }
            }
          })
          resolve(socket)
        }

        if (!this.isRelayMode()) {
          finalizeOpen()
          return
        }

        const handleRelayHandshakeMessage = (data: WebSocket.RawData) => {
          let frame: RelayConnectedFrame | RelayCloseFrame
          try {
            frame = JSON.parse(String(data)) as RelayConnectedFrame | RelayCloseFrame
          } catch (error) {
            fail("connect relay transport", error)
            return
          }
          if (frame.kind === "client_connected") {
            if (!frame.daemon_public_key) {
              fail("connect relay transport", "relay did not provide daemon public key")
              return
            }
            this.setRelayDaemonPublicKey(lane, frame.daemon_public_key)
            socket.off("message", handleRelayHandshakeMessage)
            finalizeOpen()
            return
          }
          if (frame.kind === "close") {
            fail("connect relay transport", frame.reason)
            return
          }
          fail("connect relay transport", "unexpected relay handshake frame")
        }

        socket.on("message", handleRelayHandshakeMessage)
        try {
          socket.send(JSON.stringify(buildRelayConnectFrame(this.relayAuthToken, this.relayTarget)))
        } catch (error) {
          socket.off("message", handleRelayHandshakeMessage)
          fail("write relay connect frame", error, "write_failed", true)
        }
      })

      socket.on("error", handleConnectError)
      socket.on("close", handleConnectClose)
    })

    this.setWebSocketConnectPromise(lane, nextConnectPromise)
    return nextConnectPromise
  }

  private handleWebSocketMessage(data: WebSocket.RawData, lane: KernelSocketLane) {
    let frame:
      | KernelTransportResponseFrame<unknown>
      | KernelTransportEventFrame<KernelEvent>
      | RelayResponseFrame<unknown>
      | RelayEventFrame
      | RelayCloseFrame
    try {
      frame = JSON.parse(String(data)) as
        | KernelTransportResponseFrame<unknown>
        | KernelTransportEventFrame<KernelEvent>
        | RelayResponseFrame<unknown>
        | RelayEventFrame
        | RelayCloseFrame
    } catch (error) {
      this.rejectPending(error instanceof Error ? error.message : String(error), lane)
      return
    }

    if ("type" in frame && frame.type === "event") {
      let event: KernelEvent
      try {
        event = kernelEventFromValue(frame.event)
      } catch (error) {
        this.rejectPending(error instanceof Error ? error.message : String(error), lane)
        return
      }
      this.lastReceivedEventId = frame.event_id
      this.markKernelEventReceived()
      for (const handler of this.eventHandlers) {
        handler(event)
      }
      return
    }

    if ("kind" in frame && frame.kind === "close") {
      this.rejectPending(frame.reason, lane)
      return
    }

    if ("kind" in frame && frame.kind === "client_event") {
      const subscription = this.activeKernelSubscription
      if (!subscription?.relayPrivateKey || subscription.relaySubscriptionId !== frame.subscription_id) {
        return
      }
      try {
        const decrypted = decryptRelayPayload(subscription.relayPrivateKey, frame.encrypted_event)
        const event = kernelEventFromValue(JSON.parse(decrypted))
        this.lastReceivedEventId = frame.event_id
        this.markKernelEventReceived()
        this.emitSyntheticEvent(event)
      } catch (error) {
        this.rejectPending(error instanceof Error ? error.message : String(error), lane)
      }
      return
    }

    const requestId = "type" in frame ? frame.request_id : frame.request_id
    const pending = this.pendingRequests.take(requestId)
    if (!pending) {
      return
    }

    if (frame.error) {
      pending.reject(new LocalIpcError("handle kernel response", frame.error.message, frame.error.code, frame.error.retryable))
      return
    }
    if ("kind" in frame) {
      if (!pending.relayPrivateKey) {
        pending.reject(new LocalIpcError("handle kernel response", "missing relay request key"))
        return
      }
      if (frame.encrypted_response == null) {
        pending.reject(new LocalIpcError("handle kernel response", "response envelope was empty"))
        return
      }
      try {
        const decrypted = decryptRelayPayload(pending.relayPrivateKey, frame.encrypted_response)
        pending.resolve(JSON.parse(decrypted) as unknown)
      } catch (error) {
        pending.reject(new LocalIpcError("handle kernel response", error instanceof Error ? error.message : String(error)))
      }
      return
    }
    if (frame.response == null) {
      pending.reject(new LocalIpcError("handle kernel response", "response envelope was empty"))
      return
    }

    pending.resolve(frame.response)
  }

  private rejectPending(message: string, lane?: KernelSocketLane) {
    this.pendingRequests.rejectMatching(message, lane)
  }

  private emitSyntheticEvent(event: KernelEvent) {
    for (const handler of this.eventHandlers) {
      handler(event)
    }
  }

  private clearReconnectState() {
    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout)
      this.reconnectTimeout = null
    }
    this.reconnectDelayMs = KERNEL_RECONNECT_BASE_DELAY_MS
  }

  private markKernelEventReceived() {
    this.lastKernelEventAtMs = Date.now()
    this.armKernelEventWatchdog()
  }

  private armKernelEventWatchdog() {
    this.clearKernelEventWatchdog()
    if (!this.kernelEventStaleMs || !this.activeKernelSubscription || this.eventHandlers.size === 0) {
      return
    }
    this.kernelEventWatchdog = setTimeout(() => {
      const elapsedMs = Date.now() - this.lastKernelEventAtMs
      if (!this.activeKernelSubscription || this.eventHandlers.size === 0) {
        return
      }
      if (elapsedMs < this.kernelEventStaleMs) {
        this.armKernelEventWatchdog()
        return
      }
      this.emitSyntheticEvent({
        event: "transport_closed",
        message: `kernel event stream stalled for ${elapsedMs}ms; reconnecting`,
      })
      void this.restartKernelEventStream()
    }, this.kernelEventStaleMs)
  }

  private clearKernelEventWatchdog() {
    if (this.kernelEventWatchdog) {
      clearTimeout(this.kernelEventWatchdog)
      this.kernelEventWatchdog = null
    }
  }

  private startKernelHeartbeat(socket: WebSocket, lane: KernelSocketLane) {
    this.clearKernelHeartbeat(lane)
    this.setMissedKernelPongs(lane, 0)
    const heartbeat = setInterval(() => {
      if (socket !== this.getWebSocket(lane) || socket.readyState !== WebSocket.OPEN) {
        clearInterval(heartbeat)
        return
      }
      if (this.getMissedKernelPongs(lane) >= this.kernelMaxMissedPongs) {
        if (lane === "event") {
          this.emitSyntheticEvent({
            event: "transport_closed",
            message: "kernel websocket heartbeat missed; reconnecting",
          })
        }
        this.setSuppressNextCloseEvent(lane, true)
        socket.terminate()
        this.setWebSocket(lane, null)
        this.setRelayDaemonPublicKey(lane, null)
        if (lane === "event") {
          this.scheduleReconnect()
        }
        return
      }
      this.setMissedKernelPongs(lane, this.getMissedKernelPongs(lane) + 1)
      try {
        socket.ping()
      } catch {
        if (lane === "event") {
          this.emitSyntheticEvent({
            event: "transport_closed",
            message: "kernel websocket heartbeat failed; reconnecting",
          })
        }
        this.setSuppressNextCloseEvent(lane, true)
        socket.terminate()
        this.setWebSocket(lane, null)
        this.setRelayDaemonPublicKey(lane, null)
        if (lane === "event") {
          this.scheduleReconnect()
        }
      }
    }, this.kernelPingIntervalMs)
    if (lane === "control") {
      this.controlHeartbeat = heartbeat
    } else {
      this.eventHeartbeat = heartbeat
    }
  }

  private clearKernelHeartbeat(lane: KernelSocketLane) {
    const heartbeat = lane === "control" ? this.controlHeartbeat : this.eventHeartbeat
    if (heartbeat) {
      clearInterval(heartbeat)
      if (lane === "control") {
        this.controlHeartbeat = null
      } else {
        this.eventHeartbeat = null
      }
    }
    this.setMissedKernelPongs(lane, 0)
  }

  private scheduleReconnect(delayMs = this.reconnectDelayMs) {
    if (!this.activeKernelSubscription || this.eventHandlers.size === 0 || this.reconnectTimeout) {
      return
    }

    this.reconnectTimeout = setTimeout(() => {
      this.reconnectTimeout = null
      void this.resumeKernelSubscription()
    }, this.reconnectDelayWithJitter(delayMs))
    this.reconnectDelayMs = this.nextReconnectDelayMs(delayMs)
  }

  private reconnectDelayWithJitter(delayMs: number): number {
    const boundedDelayMs = Math.max(delayMs, 0)
    if (boundedDelayMs < KERNEL_RECONNECT_BASE_DELAY_MS || this.reconnectJitterMs === 0) {
      return boundedDelayMs
    }
    const jitterMs = Math.floor(clampRandom(this.reconnectRandom()) * this.reconnectJitterMs)
    return Math.min(boundedDelayMs + jitterMs, KERNEL_RECONNECT_MAX_DELAY_MS + this.reconnectJitterMs)
  }

  private nextReconnectDelayMs(delayMs: number): number {
    return Math.min(
      Math.max(delayMs * 2, KERNEL_RECONNECT_BASE_DELAY_MS),
      KERNEL_RECONNECT_MAX_DELAY_MS,
    )
  }

  private async resumeKernelSubscription() {
    const subscription = this.activeKernelSubscription
    if (!subscription || this.eventHandlers.size === 0) {
      return
    }

    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(
          subscription.sessionId,
          subscription.attachmentId,
          this.lastReceivedEventId,
          kernelSubscriptionScopeValue(subscription),
        )
      } else {
        await this.sendWebSocket<Record<string, unknown>>(
          buildKernelSubscriptionTransportRequest(subscription, this.lastReceivedEventId),
          "event",
        )
      }
      this.clearReconnectState()
      this.markKernelEventReceived()
      this.emitSyntheticEvent({
        event: "transport_resumed",
        session_id: subscription.sessionId,
        resumed_from_event_id: this.lastReceivedEventId,
      })
    } catch {
      this.scheduleReconnect()
    }
  }

  private getWebSocket(lane: KernelSocketLane) {
    return lane === "control" ? this.controlWebsocket : this.eventWebsocket
  }

  private setWebSocket(lane: KernelSocketLane, socket: WebSocket | null) {
    if (lane === "control") {
      this.controlWebsocket = socket
    } else {
      this.eventWebsocket = socket
    }
  }

  private getConnectingWebSocket(lane: KernelSocketLane) {
    return lane === "control" ? this.connectingControlWebsocket : this.connectingEventWebsocket
  }

  private setConnectingWebSocket(lane: KernelSocketLane, socket: WebSocket | null) {
    if (lane === "control") {
      this.connectingControlWebsocket = socket
    } else {
      this.connectingEventWebsocket = socket
    }
  }

  private getWebSocketConnectPromise(lane: KernelSocketLane) {
    return lane === "control" ? this.controlWebsocketConnectPromise : this.eventWebsocketConnectPromise
  }

  private setWebSocketConnectPromise(lane: KernelSocketLane, promise: Promise<WebSocket> | null) {
    if (lane === "control") {
      this.controlWebsocketConnectPromise = promise
    } else {
      this.eventWebsocketConnectPromise = promise
    }
  }

  private getRelayDaemonPublicKey(lane: KernelSocketLane) {
    return lane === "control" ? this.controlRelayDaemonPublicKey : this.eventRelayDaemonPublicKey
  }

  private setRelayDaemonPublicKey(lane: KernelSocketLane, publicKey: string | null) {
    if (lane === "control") {
      this.controlRelayDaemonPublicKey = publicKey
    } else {
      this.eventRelayDaemonPublicKey = publicKey
    }
  }

  private getSuppressNextCloseEvent(lane: KernelSocketLane) {
    return lane === "control" ? this.suppressNextControlCloseEvent : this.suppressNextEventCloseEvent
  }

  private setSuppressNextCloseEvent(lane: KernelSocketLane, value: boolean) {
    if (lane === "control") {
      this.suppressNextControlCloseEvent = value
    } else {
      this.suppressNextEventCloseEvent = value
    }
  }

  private getMissedKernelPongs(lane: KernelSocketLane) {
    return lane === "control" ? this.missedControlPongs : this.missedEventPongs
  }

  private setMissedKernelPongs(lane: KernelSocketLane, value: number) {
    if (lane === "control") {
      this.missedControlPongs = value
    } else {
      this.missedEventPongs = value
    }
  }

  private async closeWebSocket(lane: KernelSocketLane): Promise<void> {
    const socket = this.getWebSocket(lane) ?? this.getConnectingWebSocket(lane)
    this.setWebSocket(lane, null)
    this.setConnectingWebSocket(lane, null)
    this.setWebSocketConnectPromise(lane, null)
    this.setRelayDaemonPublicKey(lane, null)
    if (!socket || socket.readyState === WebSocket.CLOSED) {
      return
    }

    await new Promise<void>((resolve) => {
      let settled = false
      let timeout: ReturnType<typeof setTimeout> | undefined
      const finish = () => {
        if (settled) {
          return
        }
        settled = true
        if (timeout) {
          clearTimeout(timeout)
        }
        resolve()
      }
      timeout = setTimeout(() => {
        socket.terminate()
        finish()
      }, IPC_WEBSOCKET_CLOSE_TIMEOUT_MS)
      this.setSuppressNextCloseEvent(lane, true)
      socket.once("close", finish)
      if (socket.readyState === WebSocket.CONNECTING) {
        socket.terminate()
      } else {
        socket.close()
      }
    })
  }

  private destroyWebSocket(lane: KernelSocketLane): void {
    const socket = this.getWebSocket(lane) ?? this.getConnectingWebSocket(lane)
    this.setWebSocket(lane, null)
    this.setConnectingWebSocket(lane, null)
    this.setWebSocketConnectPromise(lane, null)
    this.setRelayDaemonPublicKey(lane, null)
    this.clearKernelHeartbeat(lane)
    if (socket && socket.readyState !== WebSocket.CLOSED) {
      this.setSuppressNextCloseEvent(lane, true)
      socket.terminate()
    }
  }
}

function kernelEventFromValue(value: unknown): KernelEvent {
  const eventName = value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>).event
    : null
  if (typeof eventName !== "string" || !eventName.trim()) {
    throw new Error("kernel event envelope must contain a non-empty event name")
  }
  return value as KernelEvent
}

function clampRandom(value: number): number {
  if (!Number.isFinite(value)) {
    return 0
  }
  return Math.min(Math.max(value, 0), 0.999999)
}
