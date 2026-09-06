import {
  BoxRenderable,
  TextAttributes,
  TextRenderable,
} from "@opentui/core"

import type { CommandCenterItem } from "./command-center-types.js"
import { SplitBorder, theme } from "./theme.js"

type RenderCommandCenterOptions = {
  box: BoxRenderable | undefined
  renderer: ConstructorParameters<typeof BoxRenderable>[0]
  open: boolean
  items: readonly CommandCenterItem[]
  selectedIndex: number
  visibleRowCount: number
  promptHeight: number
  overlayFootprint: number
}

export function renderCommandCenterOverlay({
  box,
  renderer,
  open,
  items,
  selectedIndex,
  visibleRowCount,
  promptHeight,
  overlayFootprint,
}: RenderCommandCenterOptions): void {
  if (!box) {
    return
  }
  positionCommandCenterOverlay(box, promptHeight, overlayFootprint)
  for (const child of [...box.getChildren()]) {
    box.remove(child.id)
    child.destroyRecursively()
  }
  if (!open) {
    box.requestRender()
    return
  }

  const panel = new BoxRenderable(renderer, {
    flexDirection: "column",
    border: ["left"],
    borderColor: theme.primary,
    customBorderChars: SplitBorder.customBorderChars,
    paddingLeft: 1,
    paddingTop: 1,
    paddingBottom: 1,
    backgroundColor: theme.backgroundPanel,
    gap: 0,
  })

  const clampedIndex = Math.min(selectedIndex, Math.max(0, items.length - 1))
  const windowStart = Math.max(
    0,
    Math.min(clampedIndex - Math.floor(visibleRowCount / 2), Math.max(0, items.length - visibleRowCount)),
  )
  const visibleItems = items.slice(windowStart, windowStart + visibleRowCount)

  if (windowStart > 0) {
    panel.add(new TextRenderable(renderer, {
      content: `  ${windowStart} more above`,
      fg: theme.textMuted,
      wrapMode: "none",
    }))
  }

  for (const [offset, item] of visibleItems.entries()) {
    const index = windowStart + offset
    const selected = index === clampedIndex
    const rowColor = item.tone === "danger"
      ? theme.error
      : item.tone === "warning"
        ? theme.warning
        : theme.primary
    const row = new BoxRenderable(renderer, {
      flexDirection: "row",
      justifyContent: "space-between",
      paddingLeft: 1,
      paddingRight: 1,
      ...(selected ? { backgroundColor: rowColor } : {}),
    })
    row.add(new TextRenderable(renderer, {
      content: item.kind === "group" ? `${item.label} >` : item.label,
      fg: selected
        ? theme.background
        : item.tone === "danger"
          ? theme.error
          : item.tone === "warning"
            ? theme.warning
            : theme.text,
      attributes: selected ? TextAttributes.BOLD : TextAttributes.NONE,
      wrapMode: "none",
    }))
    row.add(new TextRenderable(renderer, {
      content: item.description,
      fg: selected ? theme.background : theme.textMuted,
      wrapMode: "none",
    }))
    panel.add(row)
  }

  const hiddenBelow = items.length - (windowStart + visibleItems.length)
  if (hiddenBelow > 0) {
    panel.add(new TextRenderable(renderer, {
      content: `  ${hiddenBelow} more below`,
      fg: theme.textMuted,
      wrapMode: "none",
    }))
  }

  box.add(panel)
  box.requestRender()
}

function positionCommandCenterOverlay(
  box: BoxRenderable,
  promptHeight: number,
  overlayFootprint: number,
): void {
  box.position = "absolute"
  box.left = 0
  box.right = 0
  box.bottom = promptHeight + overlayFootprint
  box.zIndex = 10
}
