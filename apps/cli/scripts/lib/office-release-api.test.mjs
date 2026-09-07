import assert from "node:assert/strict"
import http from "node:http"
import test from "node:test"
import { fetchOfficeRelease } from "./office-release-api.mjs"

test("cancelling a stalled response body aborts the HTTP request and leaves no listener", async t => {
  let headersSent
  const ready = new Promise(resolve => { headersSent = resolve })
  const server = http.createServer((_request, response) => {
    response.writeHead(200, { "content-type": "application/json" })
    response.flushHeaders()
    headersSent()
  })
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve))
  t.after(() => new Promise(resolve => { server.close(resolve); server.closeAllConnections() }))
  const controller = new AbortController()
  const operation = fetchOfficeRelease({ signal: controller.signal,
    fetchImpl: (_url, options) => fetch(`http://127.0.0.1:${server.address().port}`, options) })
  const rejection = assert.rejects(operation, error => error.name === "AbortError")
  await ready
  controller.abort()
  await rejection
})

test("invalid JSON is rejected without copying the upstream body into the error", async () => {
  await assert.rejects(fetchOfficeRelease({ fetchImpl: async () => new Response("untrusted-response-text") }),
    { message: "public release API returned invalid JSON" })
})

test("release API failure cancels its body without exposing the response text or retrying", async () => {
  let cancelled = false
  let calls = 0
  await assert.rejects(fetchOfficeRelease({ fetchImpl: async () => {
    calls++
    return new Response(new ReadableStream({ cancel() { cancelled = true } }), { status: 429 })
  } }), /public release API did not succeed/)
  assert.equal(calls, 1)
  assert.equal(cancelled, true)
})

test("oversized release metadata is bounded and cancelled while streaming", async () => {
  let cancelled = false
  let chunks = 0
  await assert.rejects(fetchOfficeRelease({ fetchImpl: async () => new Response(new ReadableStream({
    pull(controller) {
      if (chunks === 20) return controller.close()
      chunks++; controller.enqueue(new Uint8Array(65536).fill(32))
    },
    cancel() { cancelled = true },
  })) }), /release metadata exceeds/)
  assert.equal(cancelled, true)
  assert.ok(chunks <= 18, "must not buffer the full upstream body")
})

test("the independent release check uses one credential-free request and keeps only release metadata", async t => {
  const seen = []
  const server = http.createServer((request, response) => {
    seen.push(request.headers)
    response.writeHead(200, { "content-type": "application/json" })
    response.end(JSON.stringify({ id: 1234, tag_name: "jq-1.8.1",
      html_url: "https://github.com/jqlang/jq/releases/tag/jq-1.8.1", published_at: "2025-07-01T12:00:00Z",
      draft: false, prerelease: false, body: "irrelevant release notes", assets: [{ size: 50000000 }] }))
  })
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve))
  t.after(() => new Promise(resolve => { server.close(resolve); server.closeAllConnections() }))
  const result = await fetchOfficeRelease({ fetchImpl: (url, options) => {
    assert.equal(url, "https://api.github.com/repos/jqlang/jq/releases/latest")
    return fetch(`http://127.0.0.1:${server.address().port}`, options)
  } })
  assert.deepEqual(result, { id: 1234, tag_name: "jq-1.8.1",
    html_url: "https://github.com/jqlang/jq/releases/tag/jq-1.8.1", published_at: "2025-07-01T12:00:00Z",
    draft: false, prerelease: false })
  assert.equal(seen.length, 1)
  assert.equal(seen[0].authorization, undefined)
  assert.equal(seen[0].cookie, undefined)
  assert.equal(seen[0].accept, "application/vnd.github+json")
  assert.equal(seen[0]["x-github-api-version"], "2026-03-10")
  assert.ok(seen[0]["user-agent"])
})
