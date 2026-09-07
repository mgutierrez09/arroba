import assert from "node:assert/strict"
import http from "node:http"

// The driver supplies an independently fetched public release. No expected
// values are exposed until a correct browser submission has been accepted.
export async function startOfficeInventoryFixture({ expectedRelease, host = "127.0.0.1", publicHost = host }) {
  assert.match(publicHost, /^[a-zA-Z0-9.-]+$/)
  assert.ok(expectedRelease?.draft === false && expectedRelease.prerelease === false,
    "inventory requires a published full release")
  assert.ok(typeof expectedRelease.tag_name === "string" && expectedRelease.tag_name.length <= 256)
  assert.match(expectedRelease.tag_name, /^[a-zA-Z0-9._-]+$/)
  assert.equal(expectedRelease.html_url, `https://github.com/jqlang/jq/releases/tag/${expectedRelease.tag_name}`)
  assert.match(expectedRelease.published_at, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/)
  assert.ok(Number.isFinite(Date.parse(expectedRelease.published_at)))
  const expected = { version: expectedRelease.tag_name, release_url: expectedRelease.html_url,
    published_at: expectedRelease.published_at }
  let saved = null
  let origin
  const server = http.createServer((request, response) => {
    route(request, response).catch(() => send(response, 500, "Fixture request failed"))
  })
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, host, resolve)
  })
  origin = `http://${publicHost}:${server.address().port}`
  return { origin, async close() {
    if (server.listening) await new Promise(resolve => {
      server.close(resolve)
      server.closeAllConnections()
    })
  } }

  async function route(request, response) {
    const path = new URL(request.url ?? "/", origin).pathname
    if (request.method === "GET" && path === "/inventory") return page(response, "Software inventory", `
      <p>Find the latest published full release of <a href="https://github.com/jqlang/jq">jqlang/jq</a>.
      Use a public API through an extension you create, register, grant to yourself and invoke.
      <a href="https://docs.github.com/en/rest/releases/releases#get-the-latest-release">GitHub release API documentation</a></p>
      <p>Enter the exact tag, release page URL and publication timestamp returned by your extension.</p>
      <form method="post" action="/inventory">
        <label>Version <input name="version" required></label>
        <label>Release URL <input name="release_url" type="url" required></label>
        <label>Published at <input name="published_at" required></label>
        <button type="submit">Update inventory</button>
      </form>`)
    if (request.method === "POST" && path === "/inventory") {
      const chunks = []
      let size = 0
      for await (const chunk of request) {
        size += chunk.length
        if (size > 8192) return send(response, 413, "Request too large")
        chunks.push(chunk)
      }
      const form = new URLSearchParams(Buffer.concat(chunks).toString("utf8"))
      if (saved) return send(response, 409, "Inventory already updated")
      if (form.size !== 3 || !Object.entries(expected).every(([key, value]) =>
        form.getAll(key).length === 1 && form.get(key) === value)) {
        return send(response, 422, "Release details do not match the independent check")
      }
      saved = { status: "updated", repository: "jqlang/jq", ...expected }
      return send(response, 303, "", { location: "/inventory/receipt" })
    }
    if (request.method === "GET" && path === "/api/inventory") return send(response, 200,
      JSON.stringify(saved ?? { status: "pending" }), { "content-type": "application/json" })
    if (request.method === "GET" && path === "/inventory/receipt") {
      if (!saved) return send(response, 404, "No update yet")
      return page(response, "Inventory updated", `<p>CHARIOX_INVENTORY_UPDATED</p>
        <p>Repository: jqlang/jq</p><p>Version: ${escape(saved.version)}</p>
        <p>Release: ${escape(saved.release_url)}</p><p>Published: ${escape(saved.published_at)}</p>`)
    }
    send(response, 404, "Not found")
  }
}

function send(response, status, body, headers = {}) {
  response.writeHead(status, { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store", ...headers })
  response.end(body)
}
function page(response, title, body) {
  send(response, 200, `<!doctype html><html lang="en"><meta charset="utf-8"><title>${title}</title>
    <style>body{font:20px system-ui;max-width:850px;margin:32px}label{display:block;margin:16px 0}input{font:inherit;width:100%}button{font:inherit;padding:12px}</style>
    <h1>${title}</h1>${body}</html>`, { "content-type": "text/html; charset=utf-8" })
}
function escape(value) {
  return String(value).replace(/[&<>"']/g, char => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char])
}
