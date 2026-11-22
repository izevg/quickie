# Web Inspector

Quickie uses WebKit's Inspector for its tools and AppKit for its window and docking.
The detached Console and compact dock use the same browser appearance as the rest of
Quickie. [Design system](../system.md)

## Ownership

- AppKit owns the detached window, traffic lights, focus, fullscreen, saved frame,
  and window shadow. The detached Inspector stays nonmodal.
- WebKit's native Inspector proxy owns docking and the space reserved in the page
  host. The dock uses a 12 pt inset, a 16 pt surface radius, and one medium-elevation
  shadow. The opaque surface clips its content once.
- WebInspectorUI owns tab navigation, tool state, tables, editors, and responsive
  layouts. The Quickie adapter opens existing tools through the fork's Inspector API.
  Memory selects the Heap Allocations timeline.
- The frontend uses WebKit's existing surface, separator, syntax, and severity
  roles. The system provides the accent and label colors. Spacing follows Quickie's
  existing 4, 8, 12, 16, and 24 pt scale.

## Layout

The first local Inspector opens at the bottom when the page can accommodate it.
A user's dock or detach choice persists. An attached side reserves at least 364 pt
for the Inspector and 320 pt for the page. The 364 pt allocation leaves 340 pt inside
the Inspector at 100% zoom. The bottom dock stays available in windows where the
side dock cannot fit.

| Tool | Expected behavior |
|---|---|
| Console | Existing action bar above scrolling output and a prompt anchored below it. Warnings, errors, source links, object expansion, history, completion, and split Console remain available. |
| Network | The resource table and request details stack at 420–699 CSS px when height permits. Below 420 CSS px or with limited height, details occupy the content region and the Requests action returns to the selected row, scroll offset, sort, and filters. The wide layout keeps side-by-side details. |
| Elements | Wide bottom layout retains the DOM tree, breadcrumbs, style panels, element picker, and editor interactions. The inspected element highlight stays aligned with the page. |
| Memory | The existing Heap Allocations view retains snapshots, comparisons, imports, object graphs, and lazy retainers. |
| Timelines | The overview, range selection, recording controls, and real WebKit tracks stay available. Presentation adds no recording or animation while idle. |
| Storage | The origin navigation and editable table stay available. A selected value can be previewed in a read-only editor. Copy Value copies the complete stored string, including values longer than the preview limit. |
| Sources and other tools | Existing debugger, overflow, tab selection, extension tools, Audit, Search, and Settings remain available subject to their backend capabilities. |

## Verification

Check light and dark appearance, Increase Contrast, an alternate system accent,
active and inactive windows, and 100%, 125%, and 200% Inspector zoom. Exercise the
attached and detached Inspector with keyboard navigation and VoiceOver. Check
focus return from the Network details page and the native attachment lifecycle
when changing tabs, entering fullscreen, narrowing the window, or closing.

Measure against the same engine revision and test data. On the warmed frontend,
target at most 50 ms at p95 for prompt echo, selection, and tool switching. At 60
and 120 Hz, compare p95 frame intervals for scrolling and live resize. Where the
baseline misses the frame budget, allow at most the larger of 1 ms or 5%
regression. Network rows should remain bounded by the viewport and overscan
with both 10,000 and 100,000 requests. Check idle traces for recurring work
introduced by the presentation.
