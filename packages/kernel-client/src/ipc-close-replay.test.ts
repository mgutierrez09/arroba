import assert from "node:assert/strict"
import http from "node:http"
import type { Duplex } from "node:stream"
import test from "node:test"
import { WebSocketServer } from "ws"
import { LocalIpcClient, LocalIpcError } from "./ipc.js"

for (const shutdown of ["close", "destroy"] as const) {
  for (const phase of ["response", "backoff", "connecting"] as const) {
    test(`${shutdown} prevents replay of a request during ${phase}`, async (t) => {
      const httpServer = http.createServer()
      const server = new WebSocketServer({ noServer: true })
      const rawSockets = new Set<Duplex>()
      let connections = 0
      let requests = 0
      let reachPhase!: () => void
      const phaseReached = new Promise<void>(resolve => { reachPhase = resolve })
      httpServer.on("upgrade", (request, socket, head) => {
        rawSockets.add(socket)
        socket.once("close", () => rawSockets.delete(socket))
        connections++
        if (phase === "connecting" && connections === 1) {
          reachPhase()
          return // Hold the initial WebSocket handshake open until shutdown.
        }
        server.handleUpgrade(request, socket, head, websocket => {
          websocket.on("message", payload => {
            const frame = JSON.parse(String(payload)) as { request_id: string }
            requests++
            if (requests === 1 && phase !== "connecting") {
              if (phase === "backoff") websocket.terminate()
              else reachPhase()
              return
            }
            websocket.send(JSON.stringify({ type: "response", request_id: frame.request_id,
              response: { ok: true }, error: null }))
          })
        })
      })
      await new Promise<void>(resolve => httpServer.listen(0, "127.0.0.1", resolve))
      const address = httpServer.address()
      assert.ok(address && typeof address === "object")
      const client = new LocalIpcClient(`ws://127.0.0.1:${address.port}`, {
        controlRequestRetryDeadlineMs: 2_000, controlResponseStallMs: 1000, reconnectJitterMs: 0,
      })
      if (phase === "backoff") client.onKernelEvent(event => {
        if (event.event === "transport_closed") reachPhase()
      })
      t.after(async () => {
        client.destroy()
        for (const socket of server.clients) socket.terminate()
        for (const socket of rawSockets) socket.destroy()
        await new Promise<void>(resolve => server.close(() => resolve()))
        await new Promise<void>(resolve => httpServer.close(() => resolve()))
      })
      // Reflect rejection immediately so explicit close can settle it without
      // an unhandled-rejection race in the test itself.
      const pending = client.send({ ListSessions: null }).then(
        value => ({ value, error: null }), error => ({ value: null, error }),
      )
      await phaseReached
      await client[shutdown]()
      const outcome = await pending
      assert.ok(outcome.error instanceof LocalIpcError, "shutdown must reject, not replay and resolve")
      assert.equal(outcome.error.code, "client_closed")
      assert.equal(outcome.error.retryable, false)
      assert.equal(connections, 1, "shutdown opened a replacement connection")
      assert.equal(requests, phase === "connecting" ? 0 : 1, "shutdown replayed a request")
      // Preserve explicit reuse after shutdown, without reviving retired work.
      assert.deepEqual(await client.send({ ListSessions: null }), { ok: true })
      assert.equal(connections, 2)
      assert.equal(requests, phase === "connecting" ? 1 : 2)
    })
  }
}
