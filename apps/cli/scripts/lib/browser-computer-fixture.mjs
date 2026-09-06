import { randomUUID } from "node:crypto"
import http from "node:http"
import { parseFixtureMail } from "./browser-computer-fixture-mail.mjs"

const DEFAULT_ACCOUNT = "agent@chariox.test"
const MAX_REQUEST_BODY_BYTES = 1_048_576

export async function startBrowserComputerFixture({
  host = "127.0.0.1",
  port = 0,
  account = DEFAULT_ACCOUNT,
  password,
} = {}) {
  if (!nonEmptyString(password)) throw new Error("browser/computer fixture password is required")
  if (!nonEmptyString(account)) throw new Error("browser/computer fixture account is required")

  const sessions = new Map()
  const pendingOAuthStates = new Set()
  const oauthGrants = new Map()
  const messages = []
  const uploads = []
  const server = http.createServer(async (request, response) => {
    try {
      await routeRequest({
        request,
        response,
        account,
        password,
        sessions,
        pendingOAuthStates,
        oauthGrants,
        messages,
        uploads,
      })
    } catch (error) {
      send(response, 500, `fixture error: ${error?.message ?? String(error)}`, {
        "content-type": "text/plain; charset=utf-8",
      })
    }
  })
  await listen(server, host, port)
  const address = server.address()
  const resolvedHost = address.address === "::" ? "127.0.0.1" : address.address
  const origin = `http://${resolvedHost}:${address.port}`

  return {
    account,
    origin,
    messages,
    uploads,
    invalidateSessions: () => {
      const invalidated = sessions.size
      sessions.clear()
      return invalidated
    },
    close: async () => await closeServer(server),
  }
}

async function routeRequest({
  request,
  response,
  account,
  password,
  sessions,
  pendingOAuthStates,
  oauthGrants,
  messages,
  uploads,
}) {
  const url = new URL(request.url ?? "/", `http://${request.headers.host ?? "127.0.0.1"}`)
  const cookies = parseCookies(request.headers.cookie ?? "")
  const authenticated = sessions.get(cookies.chariox_fixture_session) === account

  if (url.pathname === "/health") {
    sendJson(response, 200, { ok: true })
    return
  }
  if (url.pathname === "/state/seed") {
    sendHtml(response, stateSeedPage(url.searchParams))
    return
  }
  if (url.pathname === "/state/check") {
    sendHtml(response, stateCheckPage())
    return
  }
  if (url.pathname === "/service-worker.js") {
    send(response, 200, serviceWorkerScript(), {
      "cache-control": "no-store",
      "content-type": "text/javascript; charset=utf-8",
      "service-worker-allowed": "/",
    })
    return
  }
  if (url.pathname === "/offline-marker") {
    send(response, 200, "CHARIOX_FIXTURE_OFFLINE_MARKER", { "content-type": "text/plain; charset=utf-8" })
    return
  }
  if (url.pathname === "/mail/login" && request.method === "GET") {
    sendHtml(response, loginPage())
    return
  }
  if (url.pathname === "/mail/login" && request.method === "POST") {
    const form = new URLSearchParams(await readBody(request))
    if (form.get("email") !== account || form.get("password") !== password) {
      sendHtml(response, loginPage("Invalid credentials"), 401)
      return
    }
    const sessionId = `fixture-${randomUUID()}`
    sessions.set(sessionId, account)
    send(response, 303, "", {
      location: "/mail/inbox",
      "set-cookie": `chariox_fixture_session=${sessionId}; Path=/; Max-Age=86400; HttpOnly; SameSite=Lax`,
    })
    return
  }
  if (url.pathname === "/oauth/start" && request.method === "GET") {
    const state = `fixture-state-${randomUUID()}`
    pendingOAuthStates.add(state)
    sendHtml(response, oauthStartPage(state))
    return
  }
  if (url.pathname === "/oauth/authorize" && request.method === "GET") {
    const state = url.searchParams.get("state") ?? ""
    if (!pendingOAuthStates.has(state)) {
      send(response, 400, "invalid OAuth state", { "content-type": "text/plain; charset=utf-8" })
      return
    }
    sendHtml(response, oauthAuthorizePage(state, account))
    return
  }
  if (url.pathname === "/oauth/authorize" && request.method === "POST") {
    const form = new URLSearchParams(await readBody(request))
    const state = form.get("state") ?? ""
    if (!pendingOAuthStates.delete(state)) {
      send(response, 400, "invalid OAuth state", { "content-type": "text/plain; charset=utf-8" })
      return
    }
    const code = `fixture-code-${randomUUID()}`
    oauthGrants.set(code, { state, account })
    send(response, 303, "", {
      location: `/oauth/callback?${new URLSearchParams({ code, state })}`,
    })
    return
  }
  if (url.pathname === "/oauth/callback" && request.method === "GET") {
    const code = url.searchParams.get("code") ?? ""
    const state = url.searchParams.get("state") ?? ""
    const grant = oauthGrants.get(code)
    if (!grant || grant.state !== state) {
      send(response, 400, "invalid OAuth callback", { "content-type": "text/plain; charset=utf-8" })
      return
    }
    oauthGrants.delete(code)
    const sessionId = `fixture-${randomUUID()}`
    sessions.set(sessionId, grant.account)
    send(response, 200, oauthCallbackPage(grant), {
      "content-type": "text/html; charset=utf-8",
      "set-cookie": `chariox_fixture_session=${sessionId}; Path=/; Max-Age=86400; HttpOnly; SameSite=Lax`,
    })
    return
  }
  if (url.pathname === "/mail/inbox") {
    if (!authenticated) return redirectToLogin(response)
    sendHtml(response, inboxPage(account, messages))
    return
  }
  if (url.pathname === "/mail/compose") {
    if (!authenticated) return redirectToLogin(response)
    sendHtml(response, composePage())
    return
  }
  if (url.pathname === "/mail/send" && request.method === "POST") {
    if (!authenticated) {
      send(response, 403, "not authenticated", { "content-type": "text/plain; charset=utf-8" })
      return
    }
    let form
    try {
      form = await parseFixtureMail(await readBodyBytes(request), request.headers["content-type"] ?? "")
    } catch (error) {
      send(response, error.code === "FIXTURE_BODY_TOO_LARGE" ? 413 : 400, "invalid fixture mail", {
        "content-type": "text/plain; charset=utf-8",
      })
      return
    }
    const message = {
      id: `message-${messages.length + 1}`,
      from: account,
      ...form,
      sentAt: new Date().toISOString(),
    }
    messages.push(message)
    send(response, 303, "", { location: `/mail/sent/${message.id}` })
    return
  }
  const sentMessageMatch = /^\/mail\/sent\/([^/]+)$/.exec(url.pathname)
  if (sentMessageMatch && request.method === "GET") {
    if (!authenticated) return redirectToLogin(response)
    const message = messages.find((candidate) => candidate.id === sentMessageMatch[1])
    if (!message) {
      send(response, 404, "message not found", { "content-type": "text/plain; charset=utf-8" })
      return
    }
    sendHtml(response, sentPage(message))
    return
  }
  if (url.pathname === "/api/messages") {
    if (!authenticated) {
      send(response, 403, "not authenticated", { "content-type": "text/plain; charset=utf-8" })
      return
    }
    sendJson(response, 200, { messages })
    return
  }
  if (url.pathname === "/interactions") {
    sendHtml(response, interactionPage())
    return
  }
  if (url.pathname === "/frames/outer") {
    sendHtml(response, outerFramePage())
    return
  }
  if (url.pathname === "/frames/inner") {
    sendHtml(response, innerFramePage())
    return
  }
  if (url.pathname === "/popup") {
    sendHtml(response, html("Fixture popup", "<h1 id=\"popup-marker\">CHARIOX_FIXTURE_POPUP</h1>"))
    return
  }
  if (url.pathname === "/downloads/sample.txt") {
    send(response, 200, "CHARIOX_FIXTURE_DOWNLOAD\n", {
      "content-disposition": "attachment; filename=chariox-fixture.txt",
      "content-type": "text/plain; charset=utf-8",
    })
    return
  }
  if (url.pathname === "/uploads" && request.method === "POST") {
    if (!authenticated) {
      send(response, 403, "not authenticated", { "content-type": "text/plain; charset=utf-8" })
      return
    }
    const body = await readBody(request)
    uploads.push({ contentType: request.headers["content-type"] ?? "", sizeBytes: Buffer.byteLength(body) })
    sendHtml(response, html("Upload complete", `<h1 id="upload-marker">CHARIOX_FIXTURE_UPLOAD ${Buffer.byteLength(body)}</h1>`))
    return
  }
  if (url.pathname === "/delayed") {
    const delayMs = Math.min(2_000, Math.max(0, Number(url.searchParams.get("ms")) || 0))
    await new Promise((resolve) => setTimeout(resolve, delayMs))
    sendHtml(response, html("Delayed marker", "<h1 id=\"delayed-marker\">CHARIOX_FIXTURE_DELAYED</h1>"))
    return
  }

  send(response, 404, "not found", { "content-type": "text/plain; charset=utf-8" })
}

function stateSeedPage(searchParams) {
  const markers = scriptJson({
    cookie: searchParams.get("cookie") ?? "fixture-cookie",
    localStorage: searchParams.get("local") ?? "fixture-local",
    indexedDb: searchParams.get("idb") ?? "fixture-idb",
    cache: searchParams.get("cache") ?? "fixture-cache",
  })
  return html("Fixture state seed", `
    <h1>Fixture state seed</h1>
    <p id="state-status">seeding</p>
    <script>
      const markers = ${markers};
      document.cookie = "chariox_fixture_state=" + encodeURIComponent(markers.cookie) + "; Path=/; Max-Age=86400; SameSite=Lax";
      localStorage.setItem("chariox_fixture_local", markers.localStorage);
      const cacheReady = caches.open("chariox-fixture-v1").then((cache) => cache.put("/cached-marker", new Response(markers.cache)));
      const workerReady = navigator.serviceWorker.register("/service-worker.js").then(() => navigator.serviceWorker.ready);
      const databaseReady = new Promise((resolve, reject) => {
        const open = indexedDB.open("chariox_fixture_state", 1);
        open.onupgradeneeded = () => open.result.createObjectStore("markers");
        open.onerror = () => reject(open.error);
        open.onsuccess = () => {
          const transaction = open.result.transaction("markers", "readwrite");
          transaction.objectStore("markers").put(markers.indexedDb, "primary");
          transaction.oncomplete = resolve;
          transaction.onerror = () => reject(transaction.error);
        };
      });
      Promise.all([cacheReady, workerReady, databaseReady]).then(() => {
        document.querySelector("#state-status").textContent = "CHARIOX_FIXTURE_STATE_SEEDED";
      });
    </script>
  `)
}

function stateCheckPage() {
  return html("Fixture state check", `
    <h1>Fixture state check</h1>
    <pre id="state-result">checking</pre>
    <script>
      const cookie = document.cookie.split("; ").find((item) => item.startsWith("chariox_fixture_state="))?.split("=")[1] ?? "";
      const database = new Promise((resolve, reject) => {
        const open = indexedDB.open("chariox_fixture_state", 1);
        open.onupgradeneeded = () => open.result.createObjectStore("markers");
        open.onerror = () => reject(open.error);
        open.onsuccess = () => {
          const get = open.result.transaction("markers", "readonly").objectStore("markers").get("primary");
          get.onsuccess = () => resolve(get.result ?? null);
          get.onerror = () => reject(get.error);
        };
      });
      Promise.all([
        database,
        caches.open("chariox-fixture-v1").then((cache) => cache.match("/cached-marker")).then((response) => response?.text() ?? null),
        navigator.serviceWorker.getRegistration(),
      ]).then(([indexedDb, cache, serviceWorker]) => {
        document.querySelector("#state-result").textContent = JSON.stringify({
          cookie: decodeURIComponent(cookie),
          localStorage: localStorage.getItem("chariox_fixture_local"),
          indexedDb,
          cache,
          serviceWorker: Boolean(serviceWorker),
        });
      });
    </script>
  `)
}

function serviceWorkerScript() {
  return `
    const CACHE = "chariox-fixture-worker-v1";
    self.addEventListener("install", (event) => event.waitUntil(caches.open(CACHE).then((cache) => cache.add("/offline-marker"))));
    self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));
    self.addEventListener("fetch", (event) => {
      if (new URL(event.request.url).pathname === "/offline-marker") {
        event.respondWith(caches.match(event.request).then((cached) => cached || fetch(event.request)));
      }
    });
  `
}

function loginPage(error = "") {
  return html("Fixture mail login", `
    <h1>Fixture mail login</h1>
    ${error ? `<p id="login-error">${escapeHtml(error)}</p>` : ""}
    <form method="post" action="/mail/login">
      <label>Email <input id="email" name="email" autocomplete="username"></label>
      <label>Password <input id="password" name="password" type="password" autocomplete="current-password"></label>
      <button id="login" type="submit">Sign in</button>
    </form>
  `)
}

function oauthStartPage(state) {
  const expectedState = scriptJson(state)
  return html("Fixture OAuth client", `
    <h1>Fixture OAuth client</h1>
    <p id="oauth-status">signed out</p>
    <a id="oauth-sign-in" href="/oauth/authorize?state=${encodeURIComponent(state)}" target="_blank" rel="opener">Sign in with Fixture</a>
    <script>
      const expectedState = ${expectedState};
      window.addEventListener("message", (event) => {
        if (event.origin !== window.location.origin || event.data?.type !== "fixture-oauth" || event.data?.state !== expectedState) return;
        document.querySelector("#oauth-status").textContent = "CHARIOX_FIXTURE_OAUTH_AUTHENTICATED " + event.data.account;
      });
    </script>
  `)
}

function oauthAuthorizePage(state, account) {
  return html("Fixture OAuth authorization", `
    <h1>Fixture OAuth authorization</h1>
    <p>Continue as ${escapeHtml(account)}</p>
    <form method="post" action="/oauth/authorize">
      <input type="hidden" name="state" value="${escapeHtml(state)}">
      <button id="oauth-authorize" type="submit">Authorize Fixture account</button>
    </form>
  `)
}

function oauthCallbackPage({ state, account }) {
  const payload = scriptJson({ type: "fixture-oauth", state, account })
  return html("Fixture OAuth callback", `
    <h1 id="oauth-callback">CHARIOX_FIXTURE_OAUTH_CALLBACK</h1>
    <button id="oauth-complete" type="button" onclick="window.close()">Complete sign-in</button>
    <script>
      window.opener?.postMessage(${payload}, window.location.origin);
    </script>
  `)
}

function inboxPage(account, messages) {
  const rows = messages.map((message) => `<li data-message-id="${escapeHtml(message.id)}">${escapeHtml(message.subject)}</li>`).join("")
  return html("Fixture inbox", `
    <h1 id="inbox-marker">CHARIOX_FIXTURE_INBOX</h1>
    <p id="account">${escapeHtml(account)}</p>
    <a id="compose" href="/mail/compose">Compose</a>
    <ul id="messages">${rows}</ul>
  `)
}

function composePage() {
  return html("Fixture compose", `
    <h1>Fixture compose</h1>
    <form method="post" action="/mail/send" enctype="multipart/form-data">
      <label>To <input id="to" name="to"></label>
      <label>Subject <input id="subject" name="subject"></label>
      <label>Body <textarea id="body" name="body"></textarea></label>
      <label>Attachment <input id="attachment" type="file" name="attachment" multiple></label>
      <button id="send" type="submit">Send</button>
    </form>
  `)
}

function sentPage(message) {
  const attachments = (message.attachments ?? []).map(file =>
    `<li>${escapeHtml(file.name)} (${file.sizeBytes} bytes)</li>`).join("")
  return html("Fixture message sent", `
    <h1 id="sent-marker">CHARIOX_FIXTURE_MESSAGE_SENT</h1>
    <p id="sent-subject">${escapeHtml(message.subject)}</p>
    <ul id="sent-attachments">${attachments}</ul>
    <a href="/mail/inbox">Inbox</a>
  `)
}

function interactionPage() {
  return html("Fixture interactions", `
    <h1>Fixture interactions</h1>
    <p id="interaction-status">ready</p>
    <div id="shadow-host"></div>
    <iframe id="outer-frame" title="Outer fixture frame" src="/frames/outer"></iframe>
    <button id="confirm" type="button">Open confirm</button>
    <button id="prompt" type="button">Open prompt</button>
    <a id="popup" href="/popup" target="_blank">Open popup</a>
    <a id="download" href="/downloads/sample.txt" download>Download fixture</a>
    <form method="post" action="/uploads" enctype="multipart/form-data">
      <input id="upload" type="file" name="fixture-file">
      <button id="upload-submit" type="submit">Upload</button>
    </form>
    <script>
      const root = document.querySelector("#shadow-host").attachShadow({ mode: "open" });
      root.innerHTML = '<label>Shadow value <input id="shadow-input"></label><button id="shadow-button">Shadow action</button>';
      root.querySelector("#shadow-button").addEventListener("click", () => {
        document.querySelector("#interaction-status").textContent = "shadow:" + root.querySelector("#shadow-input").value;
      });
      document.querySelector("#confirm").addEventListener("click", () => {
        document.querySelector("#interaction-status").textContent = confirm("Confirm fixture action?") ? "confirmed" : "dismissed";
      });
      document.querySelector("#prompt").addEventListener("click", () => {
        document.querySelector("#interaction-status").textContent = "prompt:" + (prompt("Fixture value", "default") ?? "cancelled");
      });
    </script>
  `)
}

function outerFramePage() {
  return html("Outer fixture frame", `
    <h1 id="outer-frame-marker">CHARIOX_FIXTURE_OUTER_FRAME</h1>
    <iframe id="inner-frame" title="Inner fixture frame" src="/frames/inner"></iframe>
  `)
}

function innerFramePage() {
  return html("Inner fixture frame", `
    <h1 id="inner-frame-marker">CHARIOX_FIXTURE_INNER_FRAME</h1>
    <label>Frame value <input id="frame-input"></label>
    <button id="frame-button" type="button">Frame action</button>
  `)
}

function html(title, body) {
  return `<!doctype html><html><head><meta charset="utf-8"><title>${escapeHtml(title)}</title></head><body>${body}</body></html>`
}

function redirectToLogin(response) {
  send(response, 303, "", { location: "/mail/login" })
}

function sendHtml(response, body, status = 200) {
  send(response, status, body, { "content-type": "text/html; charset=utf-8" })
}

function sendJson(response, status, value) {
  send(response, status, `${JSON.stringify(value)}\n`, { "content-type": "application/json; charset=utf-8" })
}

function send(response, status, body, headers = {}) {
  response.writeHead(status, { "cache-control": "no-store", ...headers })
  response.end(body)
}

async function readBody(request) {
  return (await readBodyBytes(request)).toString("utf8")
}

async function readBodyBytes(request) {
  const chunks = []
  let sizeBytes = 0
  for await (const chunk of request) {
    sizeBytes += chunk.length
    if (sizeBytes > MAX_REQUEST_BODY_BYTES) {
      throw Object.assign(new Error("fixture request body exceeds 1 MiB"), { code: "FIXTURE_BODY_TOO_LARGE" })
    }
    chunks.push(Buffer.from(chunk))
  }
  return Buffer.concat(chunks)
}

function parseCookies(header) {
  return Object.fromEntries(header.split(";").map((part) => {
    const separator = part.indexOf("=")
    if (separator === -1) return null
    return [part.slice(0, separator).trim(), decodeURIComponent(part.slice(separator + 1))]
  }).filter(Boolean))
}

function scriptJson(value) {
  return JSON.stringify(value).replaceAll("<", "\\u003c")
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (character) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  })[character])
}

function listen(server, host, port) {
  return new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(port, host, resolve)
  })
}

async function closeServer(server) {
  if (!server.listening) return
  await new Promise((resolve) => {
    server.close(resolve)
    server.closeAllConnections()
  })
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}
