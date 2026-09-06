import assert from "node:assert/strict"
import { test } from "node:test"
import { browserStateDrillImageConfig } from "./browser-state-drill-image.mjs"

test("the default persistence drill retains automatic image building", () => {
  assert.deepEqual(browserStateDrillImageConfig({}), ['build_image = "auto"'])
})

test("an explicit validation image disables builds without replacing the shared default", () => {
  for (const image of ["chariox-slice-linux:sandbox-validation-ac06c091c", "registry.test:5000/chariox/slice@sha256:123abc"]) {
    assert.deepEqual(browserStateDrillImageConfig({ M20_SLICE_IMAGE: image }), [
      `docker_image = "${image}"`, 'build_image = "never"',
    ])
  }
})

test("invalid explicit image references fail instead of falling back to a build", () => {
  for (const image of ["", " ", "image\n", 'image"\nbuild_image = "always', "--help"]) {
    assert.throws(() => browserStateDrillImageConfig({ M20_SLICE_IMAGE: image }), /Docker image reference/)
  }
})
