import assert from "node:assert/strict"

// A real application, not an executable/file marker standing in for one.
export function createBrowserStateEditorDrill({ containerName, runId, dockerText, dockerResult, sliceScreenWithStdin, screenshot, waitFor }) {
  const document = `/home/slice/Documents/${runId}.txt`
  const contents = `Chariox graphical editor\n${runId}\nGrüße 世界\n`
  const schema = "org.xfce.mousepad.preferences.view"
  const report = { package: "mousepad", version: "0.5.10-2", document, phases: [] }
  let display
  const userArgs = (args) => [
    "exec", "-u", "slice", "-e", `DISPLAY=${display}`, "-e", "GSETTINGS_BACKEND=keyfile", containerName, ...args,
  ]
  const user = (args, options) => dockerText(userArgs(args), options)
  const key = (value) => sliceScreenWithStdin(["computer-key-stdin", "1"], value)
  const text = (value) => sliceScreenWithStdin(["computer-type-stdin"], value)
  const readDocument = () => user(["cat", document])
  const identities = async () => ({
    version: (await user(["dpkg-query", "-W", "-f=${Version}", "mousepad"])).trim(),
    files: (await user(["sha256sum", "/usr/bin/mousepad", "/usr/share/applications/org.xfce.mousepad.desktop"])).trim(),
  })
  const windows = async () => {
    const result = await dockerResult(userArgs(["xdotool", "search", "--onlyvisible", "--class", "Mousepad"]))
    assert.ok(result.code === 0 || (result.code === 1 && !result.stderr.trim()), "editor window query failed")
    const ids = result.stdout.trim() ? result.stdout.trim().split(/\s+/) : []
    assert.ok(ids.every((id) => /^\d+$/.test(id)), "invalid editor window identity")
    return ids
  }

  async function open() {
    await dockerText(["exec", "-d", "-u", "slice", "-e", `DISPLAY=${display}`,
      "-e", "GSETTINGS_BACKEND=keyfile", containerName, "mousepad", "--disable-server", document])
    const window = await waitFor(async () => {
      const ids = await windows()
      assert.ok(ids.length <= 1, "editor launch created duplicate windows")
      return ids[0] ?? false
    }, 15_000, "Mousepad window did not appear")
    await user(["xdotool", "windowactivate", "--sync", window])
    return window
  }

  async function focused(window) {
    assert.equal((await user(["xdotool", "getactivewindow"])).trim(), window,
      "Computer input or screenshot moved focus away from the graphical editor")
  }

  async function save(expected) {
    await key("ctrl+s")
    try {
      await waitFor(async () => {
        assert.equal(await readDocument(), expected, "saved fixture document differs from typed text")
        return true
      }, 10_000, "Mousepad did not save the text entered through Computer input")
    } catch (error) {
      await screenshot("editor-save-failed").catch(() => undefined)
      throw error
    }
  }

  async function close() {
    await key("ctrl+q")
    await waitFor(async () => (await windows()).length === 0, 10_000, "Mousepad did not close")
  }

  return {
    report,
    async install() {
      await dockerText(["exec", "-u", "root", containerName, "bash", "-c", [
        "set -euo pipefail",
        "apt-get -qq update",
        "apt-get -y -qq --no-install-recommends install mousepad=0.5.10-2 libglib2.0-bin",
        "apt-get clean",
        "find /var/lib/apt/lists -mindepth 1 -delete",
      ].join("\n")])
      display = (await dockerText(["exec", containerName, "sh", "-c",
        'printf %s "${CHARIOX_SLICE_DISPLAY:-:99}"'])).trim()
      assert.match(display, /^:\d+(?:\.\d+)?$/)
      report.initialIdentity = await identities()
      assert.equal(report.initialIdentity.version, report.version)
    },
    async seed() {
      await user(["mkdir", "-p", "/home/slice/Documents"])
      await user(["touch", document])
      await user(["gsettings", "set", schema, "show-line-numbers", "true"])
      const window = await open()
      await text(contents)
      await focused(window)
      await save(contents)
      await screenshot("editor-before-save")
      await focused(window)
      await close()
      report.phases.push({ phase: "seed", typedAndSaved: true })
    },
    async verify() {
      assert.deepEqual(await identities(), report.initialIdentity, "editor binary/launcher changed during restore")
      assert.equal((await user(["gsettings", "get", schema, "show-line-numbers"])).trim(), "true",
        "the real editor preference did not survive restore")
      assert.equal(await readDocument(), contents, "the editor document did not survive restore")
      const window = await open()
      await key("ctrl+a")
      await key("ctrl+c")
      assert.equal(await sliceScreenWithStdin(["computer-clipboard-read"], ""), contents,
        "the restored graphical editor did not display the saved document")
      await key("ctrl+End")
      const suffix = "Edited after restore\n"
      await text(suffix)
      await focused(window)
      await save(contents + suffix)
      // Restore the original document before the named-backup round trips.
      await key("ctrl+a")
      await text(contents)
      await save(contents)
      await screenshot(`editor-after-restore-${report.phases.length}`)
      await focused(window)
      await close()
      await sliceScreenWithStdin(["computer-clipboard-write-stdin"], "")
      report.phases.push({ phase: "restore", identityPreserved: true, preferencePreserved: true,
        documentDisplayed: true, editedAndSaved: true, focusPreserved: true })
    },
  }
}
