import assert from "node:assert/strict"

// This is the driver's independent oracle, never an agent tool or shortcut.
export async function fetchOfficeRelease({ fetchImpl = fetch, signal } = {}) {
  const response = await fetchImpl("https://api.github.com/repos/jqlang/jq/releases/latest", {
    signal: signal ? AbortSignal.any([signal, AbortSignal.timeout(15000)]) : AbortSignal.timeout(15000),
    redirect: "error",
    headers: { accept: "application/vnd.github+json", "x-github-api-version": "2026-03-10",
      "user-agent": "chariox-office-validation" },
  })
  if (response.status !== 200) {
    await response.body?.cancel()
    throw new Error(`public release API did not succeed: HTTP ${response.status}`)
  }
  assert.ok(response.body, "public release API response has no body")
  const reader = response.body.getReader()
  const chunks = []
  let size = 0
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      size += value.byteLength
      assert.ok(size <= 1048576, "release metadata exceeds 1 MiB")
      chunks.push(value)
    }
  } finally {
    try { await reader.cancel() } finally { reader.releaseLock() }
  }
  let release
  try { release = JSON.parse(Buffer.concat(chunks).toString("utf8")) }
  catch { throw new Error("public release API returned invalid JSON") }
  return Object.fromEntries(["id", "tag_name", "html_url", "published_at", "draft", "prerelease"]
    .map(key => [key, release[key]]))
}
