# Public API office workflow

This implements office-work scenario 4 from `AGENT_SLICE_OFFICE_WORK_DRILLS.md`.
The task is to update a software inventory with the latest published full
release of `jqlang/jq`. The inventory is a disposable local application. The
release lookup must use GitHub's real public API during live acceptance.

## API research

Checked 2026-09-07 against [GitHub's release API documentation](https://docs.github.com/en/rest/releases/releases#get-the-latest-release).
`GET /repos/{owner}/{repo}/releases/latest` returns a published full release.
Public resources can be read without authentication. GitHub recommends
`Accept: application/vnd.github+json`; its current example specifies
`X-GitHub-Api-Version: 2026-03-10`. The response includes `tag_name`,
`html_url`, `published_at`, `draft`, and `prerelease`.

Use no GitHub credentials and perform no writes to GitHub. Fetch metadata
only, not release assets. A rate-limit or unavailable API is a failed live
precondition, not permission to substitute fabricated release data.

## Acceptance contract

1. Fetch the expected release independently in the owning driver. Do not put
   expected values in the provider prompt or initial inventory page.
2. The official slice-backed provider reads the inventory task in Browser mode.
3. That provider authors an extension in its workspace with its normal file
   tools, then registers an environment and the script through Chariox runtime
   tools. The driver must not author, register, grant, or invoke it for the agent.
4. The agent grants itself the extension and invokes it in the same provider
   session. Verify registration, grant, invocation, and matching output in
   complete kernel-owned history. File creation tools are allowed for authoring
   only; direct shell HTTP must not replace the extension invocation or browser.
5. The agent fills and submits the inventory through Chariox Browser tools.
   Check the resulting HTTP record against the independently fetched release,
   the provider's extension result, and the attributed Browser actions.
6. Verify the final physical screenshot, Web projection, and local/remote TUI
   action receipts. Preserve provider-session identity through registration.
7. Stop the fixture and remove owned extension state, temporary workspace,
   container, volume, listeners, and dependency links on every exit path.

The fixture's successful response alone does not prove agent-created extension
use. Kernel history and client observations remain mandatory.

## Current implementation

`apps/cli/scripts/lib/office-inventory-fixture.mjs` provides the research page,
form, receipt, and read-only result endpoint. It accepts one exact submission,
rejects incorrect, missing, duplicate, extra, or oversized fields, and does not
expose expected answers before successful submission. Invalid reference data
is rejected before opening a listener.

Run the focused HTTP tests with:

```sh
node --test apps/cli/scripts/lib/office-inventory-fixture.test.mjs apps/cli/scripts/lib/office-release-api.test.mjs
```

`office-release-api.mjs` implements the driver's independent read-only check.
It makes one unauthenticated request, rejects redirects, applies a 15-second
deadline, and limits streamed metadata to 1 MiB. Cancellation aborts a stalled
body. Error responses are cancelled without echoing their contents.

Eight focused tests pass. A live unauthenticated preflight on 2026-09-07 returned
release ID `342331441`, tag `jq-1.8.2`, published `2026-06-20T14:11:27Z`.
This is a preflight observation, not a pinned expected version for future runs.

The owning driver wiring, provider-history acceptance, live official-provider run, Web/TUI
observations, other providers, and managed-machine repeat remain outstanding.
This is not yet an end-to-end pass or benchmark evidence.
