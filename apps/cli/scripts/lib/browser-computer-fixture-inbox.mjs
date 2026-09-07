import assert from "node:assert/strict"

// In-process delivery is fixture infrastructure, never an agent mail shortcut.
// Browser readers must authenticate through the existing mail login route.
export function createFixtureInbox(account) {
  const received = []
  return {
    receiveMail(message) {
      assert.equal(message.to, account, "fixture mail recipient does not match")
      assert.ok(received.length < 100, "fixture inbox is full")
      for (const key of ["from", "subject", "body", "link", "linkLabel"]) {
        assert.ok(typeof message[key] === "string" && message[key].length > 0 && message[key].length <= 4096,
          "invalid fixture mail field")
      }
      assert.ok(["http:", "https:"].includes(new URL(message.link).protocol), "invalid fixture mail link")
      received.push({ id: `incoming-${received.length + 1}`, from: message.from,
        subject: message.subject, body: message.body, link: message.link, linkLabel: message.linkLabel })
    },
    rows(escape) {
      return received.map(mail => `<li><a href="/mail/received/${mail.id}">${escape(mail.subject)}</a></li>`).join("")
    },
    message(id, escape) {
      const mail = received.find(mail => mail.id === id)
      if (!mail) return null
      return `<h1>${escape(mail.subject)}</h1><p>From: ${escape(mail.from)}</p>
        <p>${escape(mail.body)}</p><a href="${escape(mail.link)}">${escape(mail.linkLabel)}</a>
        <p><a href="/mail/inbox">Inbox</a></p>`
    },
  }
}
