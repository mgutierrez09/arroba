import assert from "node:assert/strict"
import test from "node:test"

import {
  bindRoomEnvironmentSliceRequest,
  captureRoomEnvironmentScreenshotRequest,
  getRoomEnvironmentSliceRequest,
  cancelRoomEnvironmentActionRequest,
  getRoomEnvironmentEventsRequest,
  getRoomEnvironmentStateRequest,
  listRoomEnvironmentActionHistoryRequest,
  requestRoomEnvironmentInputTakeoverRequest,
  releaseRoomEnvironmentInputRequest,
  readRoomEnvironmentClipboardRequest,
  readRoomEnvironmentScreenshotChunkRequest,
  roomEnvironmentActionCancellationMinimumProtocolVersion,
  roomEnvironmentActionHistoryMinimumProtocolVersion,
  roomEnvironmentBrowserHistoryMinimumProtocolVersion,
  roomEnvironmentBrowserTabActionsMinimumProtocolVersion,
  roomEnvironmentEventReplayMinimumProtocolVersion,
  roomEnvironmentInputReleaseMinimumProtocolVersion,
  roomEnvironmentInputTakeoverMinimumProtocolVersion,
  roomEnvironmentLifecycleMinimumProtocolVersion,
  roomEnvironmentScreenshotMinimumProtocolVersion,
  roomEnvironmentSliceBindingMinimumProtocolVersion,
  roomEnvironmentStateMinimumProtocolVersion,
  submitRoomEnvironmentBrowserActionRequest,
  submitRoomEnvironmentActionRequest,
  startRoomEnvironmentRequest,
  stopRoomEnvironmentRequest,
  updateRoomEnvironmentViewportRequest,
  updateRoomEnvironmentPointerRequest,
  retryRoomEnvironmentRequest,
} from "./ipc-room-environment-requests.js"
import type {
  RoomEnvironmentActionCancellationOutcome,
  RoomEnvironmentActionHistoryResponse,
  RoomEnvironmentClipboardReadResponse,
  RoomEnvironmentEventsResponse,
  RoomEnvironmentStateResponse,
  RoomEnvironmentUpdatedResponse,
} from "./kernel-types-environment.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("Room browser human actions expose their exact client protocol minimums", () => {
  assert.equal(roomEnvironmentBrowserHistoryMinimumProtocolVersion, 305)
  assert.equal(roomEnvironmentBrowserTabActionsMinimumProtocolVersion, 306)
})

test("Room client capabilities expose their exact protocol minimums", () => {
  assert.deepEqual({
    state: roomEnvironmentStateMinimumProtocolVersion,
    lifecycle: roomEnvironmentLifecycleMinimumProtocolVersion,
    takeover: roomEnvironmentInputTakeoverMinimumProtocolVersion,
    release: roomEnvironmentInputReleaseMinimumProtocolVersion,
    events: roomEnvironmentEventReplayMinimumProtocolVersion,
    cancellation: roomEnvironmentActionCancellationMinimumProtocolVersion,
    history: roomEnvironmentActionHistoryMinimumProtocolVersion,
    sliceBinding: roomEnvironmentSliceBindingMinimumProtocolVersion,
    screenshot: roomEnvironmentScreenshotMinimumProtocolVersion,
  }, {
    state: 269,
    lifecycle: 270,
    takeover: 272,
    release: 273,
    events: 275,
    cancellation: 277,
    history: 279,
    sliceBinding: 282,
    screenshot: 296,
  })
})

test("Room Environment placement uses shared requests", () => {
  assert.deepEqual(bindRoomEnvironmentSliceRequest("session-1", "desktop"), {
    BindRoomEnvironmentSlice: { session_id: "session-1", slice_ref: "desktop" },
  })
  assert.deepEqual(getRoomEnvironmentSliceRequest("session-1"), {
    GetRoomEnvironmentSlice: { session_id: "session-1" },
  })
})

test("Room Environment screenshot transfer uses bounded protocol 296 requests", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 310)
  assert.deepEqual(
    captureRoomEnvironmentScreenshotRequest("session-1", "attachment-1"),
    {
      CaptureRoomEnvironmentScreenshot: {
        session_id: "session-1",
        attachment_id: "attachment-1",
      },
    },
  )
  assert.deepEqual(
    readRoomEnvironmentScreenshotChunkRequest(
      "session-1",
      "attachment-1",
      "artifact-1",
      131_072,
      131_072,
    ),
    {
      ReadRoomEnvironmentScreenshotChunk: {
        session_id: "session-1",
        attachment_id: "attachment-1",
        artifact_id: "artifact-1",
        offset: 131_072,
        max_bytes: 131_072,
      },
    },
  )
})

test("Room Environment state request matches protocol 296", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 310)
  assert.deepEqual(getRoomEnvironmentStateRequest("session-1"), {
    GetRoomEnvironmentState: {
      session_id: "session-1",
    },
  })

  const response: RoomEnvironmentStateResponse = {
    RoomEnvironmentState: {
      environment: {
        session_id: "session-1",
        environment_id: "environment-1",
        runtime_generation: 1,
        lifecycle: "ready",
        health: [
          {
            component: "browser_controller",
            state: "ready",
            diagnostic_code: null,
          },
        ],
        viewport: {
          css_width: 1280,
          css_height: 800,
          device_scale_factor: 1,
          desktop_pixel_width: 1280,
          desktop_pixel_height: 800,
          revision: 1,
          last_actor_id: "human-1",
        },
        actors: [
          {
            actor_id: "human-1",
            presentation_color: "blue",
            kind: "human",
            display_label: "Miguel",
            presence: "present",
          },
        ],
        pointers: [
          {
            actor_id: "human-1",
            x: 320,
            y: 180,
            viewport_revision: 1,
          },
        ],
        tabs: [
          {
            tab_id: "tab-1",
            url: "https://example.test/",
            title: "Example",
            document_revision: 3,
            focused: true,
          },
        ],
        focused_tab_id: "tab-1",
        actions: [
          {
            action_id: "action-1",
            sequence: 1,
            idempotency_key: "idempotency-1",
            actor_id: "human-1",
            runtime_generation: 1,
            mode: "computer",
            kind: "pointer_click",
            arguments: {
              kind: "pointer_click",
              x: 320,
              y: 180,
              button: "left",
              click_count: 1,
              viewport_revision: 1,
            },
            targets: [
              { kind: "desktop" },
              { kind: "browser_tab", id: "tab-1" },
            ],
            state: "completed",
            cancellation_requested: false,
            submitted_at_ms: 40,
            started_at_ms: 40,
            finished_at_ms: 44,
            outcome: { status: "completed" },
          },
          {
            action_id: "action-2",
            sequence: 2,
            idempotency_key: null,
            actor_id: "human-1",
            runtime_generation: 1,
            mode: "browser",
            kind: "second-click",
            targets: [{ kind: "browser_tab", id: "tab-1" }],
            state: "queued",
            cancellation_requested: false,
            submitted_at_ms: 45,
            started_at_ms: null,
            finished_at_ms: null,
            outcome: null,
          },
        ],
        input_ownership: [
          {
            target: { kind: "desktop" },
            actor_id: "human-1",
          },
        ],
        pending_input_takeovers: [],
        event_cursor: 7,
      },
    },
  }
  assert.equal(response.RoomEnvironmentState.environment.tabs[0]?.tab_id, "tab-1")
  assert.deepEqual(response.RoomEnvironmentState.environment.actions[0]?.arguments, {
    kind: "pointer_click",
    x: 320,
    y: 180,
    button: "left",
    click_count: 1,
    viewport_revision: 1,
  })
})

test("Room Environment event replay request matches protocol 296", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 310)
  assert.deepEqual(getRoomEnvironmentEventsRequest("session-1", 41), {
    GetRoomEnvironmentEvents: {
      session_id: "session-1",
      cursor: 41,
    },
  })

  const response: RoomEnvironmentEventsResponse = {
    RoomEnvironmentEvents: {
      replay: {
        Events: {
          events: [
            {
              event_id: 42,
              environment_id: "environment-1",
              runtime_generation: 3,
              kind: { ViewportChanged: { revision: 7 } },
            },
            {
              event_id: 43,
              environment_id: "environment-1",
              runtime_generation: 3,
              kind: "PointersChanged",
            },
          ],
          next_cursor: 43,
        },
      },
    },
  }
  assert.deepEqual(response.RoomEnvironmentEvents.replay, {
    Events: {
      events: [
        {
          event_id: 42,
          environment_id: "environment-1",
          runtime_generation: 3,
          kind: { ViewportChanged: { revision: 7 } },
        },
        {
          event_id: 43,
          environment_id: "environment-1",
          runtime_generation: 3,
          kind: "PointersChanged",
        },
      ],
      next_cursor: 43,
    },
  })
})

test("Room Environment Action history request matches protocol 296", () => {
  assert.deepEqual(listRoomEnvironmentActionHistoryRequest("session-1", 42, 25), {
    ListRoomEnvironmentActionHistory: {
      session_id: "session-1",
      before_sequence: 42,
      limit: 25,
    },
  })

  const response: RoomEnvironmentActionHistoryResponse = {
    RoomEnvironmentActionHistoryListed: {
      page: {
        actions: [],
        next_before_sequence: null,
      },
    },
  }
  assert.deepEqual(response.RoomEnvironmentActionHistoryListed.page.actions, [])
})

test("Room Environment start request keeps viewport ownership at the kernel seam", () => {
  assert.deepEqual(
    startRoomEnvironmentRequest("session-1", {
      css_width: 1280,
      css_height: 800,
      device_scale_factor: 2,
      desktop_pixel_width: 2560,
      desktop_pixel_height: 1600,
    }),
    {
      StartRoomEnvironment: {
        session_id: "session-1",
        viewport: {
          css_width: 1280,
          css_height: 800,
          device_scale_factor: 2,
          desktop_pixel_width: 2560,
          desktop_pixel_height: 1600,
        },
      },
    },
  )

  const response: RoomEnvironmentUpdatedResponse = {
    RoomEnvironmentUpdated: {
      environment: {
        session_id: "session-1",
        environment_id: "environment-session-1",
        runtime_generation: 1,
        lifecycle: "starting",
        health: [],
        viewport: {
          css_width: 1280,
          css_height: 800,
          device_scale_factor: 2,
          desktop_pixel_width: 2560,
          desktop_pixel_height: 1600,
          revision: 1,
          last_actor_id: null,
        },
        actors: [],
        pointers: [],
        tabs: [],
        focused_tab_id: null,
        actions: [],
        input_ownership: [],
        pending_input_takeovers: [],
        event_cursor: 1,
      },
    },
  }
  assert.equal(response.RoomEnvironmentUpdated.environment.lifecycle, "starting")
})

test("Room Environment stop request uses the shared lifecycle seam", () => {
  assert.deepEqual(stopRoomEnvironmentRequest("session-1"), {
    StopRoomEnvironment: {
      session_id: "session-1",
    },
  })
})

test("Room Environment retry request uses the shared lifecycle seam", () => {
  assert.deepEqual(retryRoomEnvironmentRequest("session-1"), {
    RetryRoomEnvironment: {
      session_id: "session-1",
    },
  })
})

test("Room Environment viewport update carries only dimensions and observed revision", () => {
  assert.deepEqual(
    updateRoomEnvironmentViewportRequest(
      "session-1",
      4,
      {
        css_width: 1440,
        css_height: 900,
        device_scale_factor: 2,
        desktop_pixel_width: 2880,
        desktop_pixel_height: 1800,
      },
    ),
    {
      UpdateRoomEnvironmentViewport: {
        session_id: "session-1",
        expected_revision: 4,
        viewport: {
          css_width: 1440,
          css_height: 900,
          device_scale_factor: 2,
          desktop_pixel_width: 2880,
          desktop_pixel_height: 1800,
        },
      },
    },
  )
})

test("Room Environment pointer update carries observed generations but no Actor identity", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 310)
  assert.deepEqual(updateRoomEnvironmentPointerRequest("session-1", 3, 7, { x: 320, y: 180 }), {
    UpdateRoomEnvironmentPointer: {
      session_id: "session-1",
      runtime_generation: 3,
      viewport_revision: 7,
      pointer: { x: 320, y: 180 },
    },
  })
  assert.deepEqual(updateRoomEnvironmentPointerRequest("session-1", 3, 7, null), {
    UpdateRoomEnvironmentPointer: {
      session_id: "session-1",
      runtime_generation: 3,
      viewport_revision: 7,
      pointer: null,
    },
  })
})

test("Room Environment takeover request cannot forge Actor identity", () => {
  assert.deepEqual(
    requestRoomEnvironmentInputTakeoverRequest("session-1", { kind: "desktop" }),
    {
      RequestRoomEnvironmentInputTakeover: {
        session_id: "session-1",
        target: { kind: "desktop" },
      },
    },
  )
})

test("Room Environment input release request cannot forge Actor identity", () => {
  assert.deepEqual(
    releaseRoomEnvironmentInputRequest("session-1", { kind: "desktop" }),
    {
      ReleaseRoomEnvironmentInput: {
        session_id: "session-1",
        target: { kind: "desktop" },
      },
    },
  )
})

test("Room Environment Action cancellation request cannot forge Actor identity", () => {
  assert.deepEqual(cancelRoomEnvironmentActionRequest("session-1", "action-7"), {
    CancelRoomEnvironmentAction: {
      session_id: "session-1",
      action_id: "action-7",
    },
  })

  const outcome: RoomEnvironmentActionCancellationOutcome = {
    state: "cancellation_requested",
  }
  assert.equal(outcome.state, "cancellation_requested")
})

test("Room Environment pointer click submission carries observed generations but no Actor identity", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 310)
  assert.deepEqual(
    submitRoomEnvironmentActionRequest("session-1", 4, 9, "input-1", {
      kind: "pointer_click",
      x: 320,
      y: 180,
      button: "left",
      click_count: 1,
    }),
    {
      SubmitRoomEnvironmentAction: {
        session_id: "session-1",
        runtime_generation: 4,
        viewport_revision: 9,
        idempotency_key: "input-1",
        action: {
          kind: "pointer_click",
          x: 320,
          y: 180,
          button: "left",
          click_count: 1,
        },
      },
    },
  )

})

test("Room Environment browser history uses a stable tab without an Actor identity", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 310)
  assert.deepEqual(
    submitRoomEnvironmentBrowserActionRequest("session-1", 4, "history-back-1", {
      kind: "history",
      tab_id: "tab-7",
      action: "back",
    }),
    {
      SubmitRoomEnvironmentBrowserAction: {
        session_id: "session-1",
        runtime_generation: 4,
        idempotency_key: "history-back-1",
        action: {
          kind: "history",
          tab_id: "tab-7",
          action: "back",
        },
      },
    },
  )
})

test("Room Environment browser tab lifecycle uses a stable tab without an Actor identity", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 310)
  for (const action of ["activate", "close"] as const) {
    assert.deepEqual(
      submitRoomEnvironmentBrowserActionRequest("session-1", 4, `tab-${action}-1`, {
        kind: "tab",
        tab_id: "tab-7",
        action,
      }),
      {
        SubmitRoomEnvironmentBrowserAction: {
          session_id: "session-1",
          runtime_generation: 4,
          idempotency_key: `tab-${action}-1`,
          action: {
            kind: "tab",
            tab_id: "tab-7",
            action,
          },
        },
      },
    )
  }
})

test("Room Environment pointer motion submissions carry canonical desktop coordinates", () => {
  assert.deepEqual(
    submitRoomEnvironmentActionRequest("session-1", 4, 9, "input-move-1", {
      kind: "pointer_move",
      x: 640,
      y: 400,
    }),
    {
      SubmitRoomEnvironmentAction: {
        session_id: "session-1",
        runtime_generation: 4,
        viewport_revision: 9,
        idempotency_key: "input-move-1",
        action: {
          kind: "pointer_move",
          x: 640,
          y: 400,
        },
      },
    },
  )
  assert.deepEqual(
    submitRoomEnvironmentActionRequest("session-1", 4, 9, "input-drag-1", {
      kind: "pointer_drag",
      from_x: 120,
      from_y: 160,
      to_x: 720,
      to_y: 560,
      button: "left",
    }),
    {
      SubmitRoomEnvironmentAction: {
        session_id: "session-1",
        runtime_generation: 4,
        viewport_revision: 9,
        idempotency_key: "input-drag-1",
        action: {
          kind: "pointer_drag",
          from_x: 120,
          from_y: 160,
          to_x: 720,
          to_y: 560,
          button: "left",
        },
      },
    },
  )
  assert.deepEqual(
    submitRoomEnvironmentActionRequest("session-1", 4, 9, "input-scroll-1", {
      kind: "pointer_scroll",
      x: 640,
      y: 400,
      horizontal_steps: -3,
      vertical_steps: 5,
    }),
    {
      SubmitRoomEnvironmentAction: {
        session_id: "session-1",
        runtime_generation: 4,
        viewport_revision: 9,
        idempotency_key: "input-scroll-1",
        action: {
          kind: "pointer_scroll",
          x: 640,
          y: 400,
          horizontal_steps: -3,
          vertical_steps: 5,
        },
      },
    },
  )
})

test("Room Environment keyboard submissions preserve text, chords, and repeat counts", () => {
  assert.deepEqual(
    submitRoomEnvironmentActionRequest("session-1", 4, 9, "input-text-1", {
      kind: "keyboard_text",
      text: "Grüße 世界",
    }),
    {
      SubmitRoomEnvironmentAction: {
        session_id: "session-1",
        runtime_generation: 4,
        viewport_revision: 9,
        idempotency_key: "input-text-1",
        action: {
          kind: "keyboard_text",
          text: "Grüße 世界",
        },
      },
    },
  )
  assert.deepEqual(
    submitRoomEnvironmentActionRequest("session-1", 4, 9, "input-key-1", {
      kind: "keyboard_key",
      key: "ctrl+shift+p",
      repeat: 3,
    }),
    {
      SubmitRoomEnvironmentAction: {
        session_id: "session-1",
        runtime_generation: 4,
        viewport_revision: 9,
        idempotency_key: "input-key-1",
        action: {
          kind: "keyboard_key",
          key: "ctrl+shift+p",
          repeat: 3,
        },
      },
    },
  )
})

test("Room Environment clipboard requests use protocol 303 without accepting Actor identity", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 310)
  assert.deepEqual(
    submitRoomEnvironmentActionRequest("session-1", 4, 9, "clipboard-1", {
      kind: "clipboard_write",
      text: "Clipboard Grüße 世界",
    }),
    {
      SubmitRoomEnvironmentAction: {
        session_id: "session-1",
        runtime_generation: 4,
        viewport_revision: 9,
        idempotency_key: "clipboard-1",
        action: {
          kind: "clipboard_write",
          text: "Clipboard Grüße 世界",
        },
      },
    },
  )
  assert.deepEqual(readRoomEnvironmentClipboardRequest("session-1", 4), {
    ReadRoomEnvironmentClipboard: {
      session_id: "session-1",
      runtime_generation: 4,
    },
  })
  const response: RoomEnvironmentClipboardReadResponse = {
    RoomEnvironmentClipboardRead: { content: "Clipboard Grüße 世界" },
  }
  assert.equal(response.RoomEnvironmentClipboardRead.content, "Clipboard Grüße 世界")
})
