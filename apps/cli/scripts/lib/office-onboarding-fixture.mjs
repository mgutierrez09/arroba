import assert from "node:assert/strict"
import { randomBytes, randomUUID, scrypt, timingSafeEqual } from "node:crypto"
import http from "node:http"
import { promisify } from "node:util"

const deriveKey = promisify(scrypt)

// A separate local SaaS origin for the office drill. Only mail delivery is
// fixture-owned; registration, mailbox access and confirmation are browser work.
export async function startOfficeOnboardingFixture({ mail, host = "127.0.0.1", publicHost = host, now = Date.now } = {}) {
  assert.ok(mail?.account && typeof mail.receiveMail === "function", "onboarding requires a mail fixture")
  assert.match(publicHost, /^[a-zA-Z0-9.-]+$/, "invalid onboarding public host")
  let account = null
  let registering = false
  let confirmationToken = null
  let expiresAt = 0
  let session = null
  let origin

  const server = http.createServer((request, response) => {
    route(request, response).catch(() => send(response, 500, "Fixture request failed"))
  })
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, host, resolve)
  })
  origin = `http://${publicHost}:${server.address().port}`
  return {
    origin,
    async close() {
      if (server.listening) await new Promise(resolve => {
        server.close(resolve)
        server.closeAllConnections()
      })
      account?.key.fill(0)
      account = null
      session = null
      confirmationToken = null
    },
  }

  async function route(request, response) {
    const url = new URL(request.url ?? "/", origin)
    const method = request.method
    if (method === "GET" && url.pathname === "/service/register") {
      return page(response, "Register for Office Service", `
        <form method="post" action="/service/register">
          <label>Email <input name="email" autocomplete="username" type="email" required></label>
          <label>Organization <input name="organization" required></label>
          <label>Password <input name="password" type="password" autocomplete="new-password" required></label>
          <button type="submit">Create account</button>
        </form>`)
    }
    if (method === "POST" && url.pathname === "/service/register") {
      const form = await readForm(request, response)
      if (!form) return
      if (form.get("email") !== mail.account || !form.get("organization")?.trim()
        || form.get("organization").length > 160 || (form.get("password")?.length ?? 0) < 12
        || form.get("password").length > 1024) return send(response, 400, "Invalid registration")
      if (account || registering) return send(response, 409, "Account already registered")
      registering = true
      try {
        const salt = randomBytes(16)
        const key = await deriveKey(form.get("password"), salt, 32)
        const token = randomUUID()
        mail.receiveMail({ to: mail.account, from: "signup@office.chariox.test",
          subject: "Confirm your Office Service account", body: "Confirm your email to finish onboarding.",
          link: `${origin}/service/confirm?token=${token}`, linkLabel: "Confirm account" })
        account = { id: "service-account-1", email: mail.account, organization: form.get("organization").trim(),
          salt, key, confirmed: false }
        confirmationToken = token
        expiresAt = now() + 5 * 60 * 1000
      } finally { registering = false }
      return redirect(response, "/service/check-email")
    }
    if (method === "GET" && url.pathname === "/service/check-email") {
      return page(response, "Check your email", "<p>Open your inbox and confirm your email before signing in.</p>")
    }
    if (["GET", "POST"].includes(method) && url.pathname === "/service/confirm") {
      if (!confirmationToken || url.searchParams.get("token") !== confirmationToken) {
        return send(response, 400, "Invalid or used confirmation")
      }
      if (now() >= expiresAt) return send(response, 410, "Confirmation expired")
      if (method === "GET") return page(response, "Confirm your email", `
        <form method="post" action="/service/confirm?token=${confirmationToken}">
          <button type="submit">Confirm account</button>
        </form>`)
      confirmationToken = null
      account.confirmed = true
      return signIn(response)
    }
    if (method === "GET" && url.pathname === "/service/login") {
      return page(response, "Office Service login", `
        <form method="post" action="/service/login">
          <label>Email <input name="email" autocomplete="username"></label>
          <label>Password <input name="password" type="password" autocomplete="current-password"></label>
          <button type="submit">Sign in</button>
        </form>`)
    }
    if (method === "POST" && url.pathname === "/service/login") {
      const form = await readForm(request, response)
      if (!form) return
      if (!account || form.get("email") !== account.email || !form.get("password")
        || form.get("password").length > 1024) return send(response, 401, "Invalid credentials")
      const key = await deriveKey(form.get("password"), account.salt, 32)
      const matches = timingSafeEqual(key, account.key)
      key.fill(0)
      if (!matches) return send(response, 401, "Invalid credentials")
      if (!account.confirmed) return send(response, 403, "Confirm your email first")
      return signIn(response)
    }
    if (method === "GET" && ["/service/dashboard", "/api/account"].includes(url.pathname)) {
      const cookie = (request.headers.cookie ?? "").split(";").map(value => value.trim())
        .find(value => value.startsWith("chariox_office_session="))?.slice("chariox_office_session=".length)
      if (!session || cookie !== session || !account?.confirmed) return send(response, 401, "Sign in first")
      const metadata = { id: account.id, email: account.email, organization: account.organization, status: "active" }
      if (url.pathname === "/api/account") return send(response, 200, JSON.stringify(metadata), {
        "content-type": "application/json; charset=utf-8" })
      return page(response, "Onboarding complete", `<p id="onboarding-marker">CHARIOX_FIXTURE_ONBOARDING_COMPLETE</p>
        <p>Account: ${escape(metadata.id)}</p><p>Email: ${escape(metadata.email)}</p>
        <p>Organization: ${escape(metadata.organization)}</p>`)
    }
    send(response, 404, "Not found")
  }

  function signIn(response) {
    session = randomUUID()
    redirect(response, "/service/dashboard", {
      "set-cookie": `chariox_office_session=${session}; Path=/; HttpOnly; SameSite=Lax`,
    })
  }
}

async function readForm(request, response) {
  const chunks = []
  let size = 0
  for await (const chunk of request) {
    size += chunk.length
    if (size > 8192) { send(response, 413, "Request too large"); return null }
    chunks.push(chunk)
  }
  return new URLSearchParams(Buffer.concat(chunks).toString("utf8"))
}

function send(response, status, body, headers = {}) {
  response.writeHead(status, { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store",
    "referrer-policy": "no-referrer", ...headers })
  response.end(body)
}
function redirect(response, location, headers = {}) { send(response, 303, "", { location, ...headers }) }
function page(response, title, body) {
  send(response, 200, `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title></head>
    <body><h1>${title}</h1>${body}</body></html>`, { "content-type": "text/html; charset=utf-8" })
}
function escape(value) {
  return String(value).replace(/[&<>"']/g, ch => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[ch])
}
