import assert from "node:assert/strict"
import test from "node:test"
import { startOfficeInventoryFixture } from "./office-inventory-fixture.mjs"

const release = { tag_name: "jq-1.8.1", html_url: "https://github.com/jqlang/jq/releases/tag/jq-1.8.1",
  published_at: "2025-07-01T12:00:00Z", draft: false, prerelease: false }
const fields = { version: release.tag_name, release_url: release.html_url, published_at: release.published_at }
const submit = (origin, values) => fetch(`${origin}/inventory`, { method: "POST", redirect: "manual",
  body: new URLSearchParams(values) })

test("inventory refuses an invalid oracle instead of accepting guessed or non-public release data", async () => {
  for (const expectedRelease of [undefined, {}, { ...release, draft: true }, { ...release, prerelease: true },
    { ...release, tag_name: "" }, { ...release, tag_name: "x".repeat(300) },
    { ...release, html_url: "https://example.com/release" },
    { ...release, html_url: "https://github.com/jqlang/jq/releases/tag/wrong" },
    { ...release, published_at: "not a timestamp" },
  ]) {
    let fixture
    try {
      await assert.rejects(async () => { fixture = await startOfficeInventoryFixture({ expectedRelease }) })
    } finally { await fixture?.close() }
  }
})

test("inventory rejects wrong, partial, ambiguous and oversized submissions without changing the record", async t => {
  const fixture = await startOfficeInventoryFixture({ expectedRelease: release })
  t.after(() => fixture.close())
  for (const values of [
    { ...fields, version: "outdated" }, { ...fields, release_url: "https://example.com" },
    { ...fields, published_at: "yesterday" }, { version: fields.version },
    [...Object.entries(fields), ["version", "another-version"]],
    { ...fields, unexpected: "ignored input is not allowed" },
  ]) {
    assert.equal((await submit(fixture.origin, values)).status, 422)
    assert.deepEqual(await fetch(`${fixture.origin}/api/inventory`).then(r => r.json()), { status: "pending" })
    assert.equal((await fetch(`${fixture.origin}/inventory/receipt`)).status, 404)
  }
  assert.equal((await submit(fixture.origin, { ...fields, version: "x".repeat(9000) })).status, 413)
  const results = await Promise.all([submit(fixture.origin, fields), submit(fixture.origin, fields)])
  assert.deepEqual(results.map(r => r.status).sort(), [303, 409])
})

test("inventory reveals the research task, accepts the independently checked release, and shows the saved result", async t => {
  const fixture = await startOfficeInventoryFixture({ expectedRelease: release })
  t.after(() => fixture.close())
  const page = await fetch(`${fixture.origin}/inventory`).then(r => r.text())
  assert.match(page, /https:\/\/github.com\/jqlang\/jq/)
  assert.match(page, /docs.github.com/)
  assert.ok(!page.includes(release.tag_name), "the fixture must not hand the expected answer to the agent")
  assert.deepEqual(await fetch(`${fixture.origin}/api/inventory`).then(r => r.json()), { status: "pending" })
  assert.equal((await submit(fixture.origin, fields)).status, 303)
  assert.deepEqual(await fetch(`${fixture.origin}/api/inventory`).then(r => r.json()), {
    status: "updated", repository: "jqlang/jq", ...fields,
  })
  const receipt = await fetch(`${fixture.origin}/inventory/receipt`).then(r => r.text())
  assert.match(receipt, /CHARIOX_INVENTORY_UPDATED/)
  assert.match(receipt, /jq-1.8.1/)
  assert.equal((await submit(fixture.origin, fields)).status, 409)
  await fixture.close()
  await assert.rejects(fetch(`${fixture.origin}/inventory`))
})
