# Drag & Drop Tab Reordering and Pane Splitting

> **Historical design record:** This page captures earlier design or implementation work and is not current guidance. Verify behavior against the [documentation index](../../README.md), the [developer guide](../../../CLAUDE.md), and the current Rust source.

**Date:** 2026-04-07
**Status:** Approved

## Overview

Unified drag-and-drop system for terminal tabs in wmux. One drag gesture supports three actions: reorder tabs within the same pane, move tabs between panes, and split panes by dropping on edges. VS Code-style edge drop zones with visual feedback.

## Current State

The codebase already has partial DnD:
- `SurfaceTabBar.tsx`: tabs are `draggable`, `onDragStart` sets `application/wmux-surface` data (sourcePaneId + surfaceId), `onDrop` calls `onDropSurface`
- `PaneWrapper.tsx`: has `handleDropSurface` that calls `moveSurface()`
- `surface-slice.ts`: `moveSurface(workspaceId, sourcePaneId, surfaceId, targetPaneId)` is fully implemented
- CSS: `surface-tab--drag-over` class exists in TSX but has zero CSS styles

What's missing: reorder within pane, edge drop zones for splitting, all visual feedback.

## Design

### Three Actions, One Drag

| Drop Target | Action | Store Operation |
|-------------|--------|-----------------|
| Between tabs (same pane) | Reorder | `reorderSurface(wsId, paneId, surfaceId, newIndex)` (new) |
| Center zone of another pane | Move tab | `moveSurface()` (exists) |
| Edge zone (left/right/top/bottom) | Split pane | `splitAndMoveSurface()` (new) |

### Drop Zone Geometry (VS Code style)

When a tab is being dragged, each pane displays 5 drop zones:

```
+---------------------------+
|           TOP (25%)       |
+------+-------------+-----+
|      |              |     |
| LEFT |   CENTER     |RIGHT|
| 25%  |   (green)    | 25% |
|      |              |     |
+------+-------------+-----+
|          BOTTOM (25%)     |
+---------------------------+
```

- **Edge zones** (LEFT/RIGHT/TOP/BOTTOM): blue highlight (`rgba(137,180,250,0.15)`) with 2px blue border on the split side
- **Center zone**: green highlight (`rgba(166,227,161,0.12)`) with 2px green border — means "add as tab"
- Zones appear on ALL panes simultaneously when a drag starts
- The zone under the cursor gets the highlight; others stay invisible until hovered

### Visual Feedback

| Element | During Drag |
|---------|-------------|
| Source tab | `opacity: 0.3`, `border: 1px dashed #6c7086` |
| Drop zone (edge, hovered) | Blue overlay + border |
| Drop zone (center, hovered) | Green overlay + border |
| Tab insertion marker (reorder) | 2px blue vertical bar between tabs |
| Cursor | `cursor: grabbing` on body during drag |
| Split preview | The hovered edge zone shows a 50/50 split preview |

### Drag Start

When the user starts dragging a tab:
1. Set drag data: `application/wmux-surface` with `{ sourcePaneId, surfaceId }`
2. Set the source tab to `opacity: 0.3` with dashed border
3. Add `cursor: grabbing` to body
4. Show drop zones on all panes (via a global `isDragging` state)

### Reorder Logic (same pane)

When dragging over tabs in the same pane:
- Calculate insertion index based on cursor X position relative to tab boundaries
- Show a 2px blue vertical bar at the insertion point
- On drop: call `reorderSurface(workspaceId, paneId, surfaceId, newIndex)`

### Move Logic (center zone, different pane)

Existing behavior — drop on center zone of another pane calls `moveSurface()`. The only change is adding the green center zone visual.

### Split Logic (edge zones)

When dropping on an edge zone of any pane (including the source pane):
1. Determine direction from the zone: left → horizontal (new pane left), right → horizontal (new pane right), top → vertical (new pane top), bottom → vertical (new pane bottom)
2. Call new store method `splitAndMoveSurface(workspaceId, targetPaneId, sourcePaneId, surfaceId, direction)`
3. This method:
   - Creates a new pane via `splitNode(tree, targetPaneId, newPaneId, 'terminal', direction)`
   - Removes the surface from source pane
   - Adds the surface to the new pane
   - If source pane becomes empty, removes it via `removeLeaf()`

### Edge Cases

- **Last tab in a pane**: Dragging the only tab of a pane to an edge of another pane should work. After the move, the source pane (now empty) is removed.
- **Drop on own pane's edge**: Should split the pane. The tab moves to the new half.
- **Drop outside any zone**: Cancel — tab returns to original position, no action.
- **Browser/markdown tabs**: All surface types are draggable, not just terminals.

## Files to Modify

| File | Changes |
|------|---------|
| `src/renderer/store/surface-slice.ts` | Add `reorderSurface()` and `splitAndMoveSurface()` |
| `src/renderer/components/SplitPane/SurfaceTabBar.tsx` | Reorder within pane, insertion marker, drag ghost style |
| `src/renderer/components/SplitPane/PaneWrapper.tsx` | Edge drop zones (4 edges + center), zone detection, split on drop |
| `src/renderer/styles/splitpane.css` | All drag feedback styles (zones, markers, opacity, cursors) |
| `src/renderer/store/index.ts` | Expose new store methods (if needed) |

## What We Don't Build

- Custom drag image / ghost preview (use browser default)
- Drag to create new window (out of scope)
- Keyboard-based tab moving (shortcuts — separate feature)
- Animation on drop (instant layout change is fine)
