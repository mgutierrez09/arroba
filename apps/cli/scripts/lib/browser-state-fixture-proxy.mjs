// Test-only TCP bridge. Chromium can use its ordinary loopback secure context
// while the host fixture retains the message ledger used by the M20 driver.
// Self-contained because the driver executes this function in the test slice.
export async function startBrowserStateFixtureProxy({ port, upstreamHost, upstreamPort }) {
  const net = await import("node:net")
  const sockets = new Set()
  const server = net.createServer((incoming) => {
    const outgoing = net.connect({ host: upstreamHost, port: upstreamPort })
    sockets.add(incoming)
    sockets.add(outgoing)
    const close = () => { incoming.destroy(); outgoing.destroy() }
    for (const socket of [incoming, outgoing]) {
      socket.setTimeout(10_000, close)
      socket.on("error", close)
      socket.once("close", () => { sockets.delete(socket); close() })
    }
    incoming.pipe(outgoing).pipe(incoming)
  })
  server.maxConnections = 16
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(port, "127.0.0.1", resolve)
  })
  return {
    address: server.address(),
    close: () => new Promise((resolve, reject) => {
      for (const socket of sockets) socket.destroy()
      server.close(error => error ? reject(error) : resolve())
    }),
  }
}
