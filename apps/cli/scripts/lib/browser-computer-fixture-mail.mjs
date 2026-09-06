import { createHash } from "node:crypto"

export async function parseFixtureMail(body, contentType) {
  const multipart = /^multipart\/form-data(?:;|$)/i.test(contentType)
  const form = multipart
    ? await new Response(body, { headers: { "content-type": contentType } }).formData()
    : new URLSearchParams(body.toString("utf8"))
  const attachments = []
  for (const field of ["to", "subject", "body"]) {
    const values = form.getAll(field)
    if (values.length > 1 || values.some(value => typeof value !== "string")) {
      throw new Error("mail fields must be single text values")
    }
  }
  if (multipart) {
    for (const file of form.getAll("attachment")) {
      // Browsers send an empty unnamed part for an unselected file input.
      if (file === "" || (!file.name && file.size === 0)) continue
      if (typeof file === "string") throw new Error("attachment must be a file")
      if (attachments.length === 20) throw new Error("mail accepts at most 20 attachments")
      attachments.push({ name: file.name, contentType: file.type, sizeBytes: file.size,
        sha256: createHash("sha256").update(Buffer.from(await file.arrayBuffer())).digest("hex") })
    }
  }
  return { to: form.get("to") ?? "", subject: form.get("subject") ?? "", body: form.get("body") ?? "",
    ...(attachments.length ? { attachments } : {}) }
}
