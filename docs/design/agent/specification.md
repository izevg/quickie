# Agent workspace

Quickie opens the Agent panel in the source browser window from the palette or the agent
shortcut. The source page stays visible and interactive beside the conversation, tool activity,
and composer. The panel stays visible until the user closes it or closes its host window.
Opening the panel sends no request.

## Working with a page

1. The workspace identifies the source tab and origin. A prompt can refer to that page
   or to the current page in the workspace.
2. The user sends a prompt from the composer or the palette's single agent action. The
   model can answer, ask a follow-up, or request an available browser tool. Existing skills
   supply instructions to the same conversation.
3. The agent can read the source page. It uses the isolated Agent profile for operations
   that drive a page. When an isolated navigation succeeds, the window shows its preview
   beside the panel. The panel switches between that preview and the source page.
4. After each tool result, the model can use the result in its answer or decide on another
   step. The panel remains available while it works and after it finishes.
5. Stop cancels the turn. Closing the workspace cancels the turn and drops the ephemeral
   browsing session and conversation.

## Panel

- Place one rounded E2 glass panel at the top right of the source window. The live page
  stays interactive beside it. Inset the panel 16 points from the top and 24 points from
  the right, with a 16-point gutter beside the page. The panel takes 40% of the window width
  within 320–480 points. It starts 240 points tall and grows with the conversation to the
  window height minus its insets, at most 640 points.
- Keep the source label and Stop above the transcript. Keep a single composer at the
  bottom, with the endpoint setup action when needed. Follow new rows when the reader
  is at the end and preserve their scroll position when they have scrolled up.
- Reuse the answer typography and Markdown. Show verified sources with each answer.
  When a citation's passage becomes unavailable, mark that source in place on activation.
- Insert each actual tool invocation into the conversation in order. A row displays
  the operation, its target when available, and its running, completed, failed, cancelled,
  or approval state. Keep detail in the disclosure row and preserve terminal failures and
  stops in the transcript.
- Tools that require approval must present the exact operation and destination and wait
  for the decision. The initial read and same-origin navigation tools use provenance checks.
- Calls and statuses are readable with VoiceOver and keyboard navigation. Text and icons
  communicate state alongside the existing accent colors.

## Automation contract

Use the configured API base and in-process WebKit Automation transport. Expose typed
tools with a name, bounded arguments, and typed results. The initial tool set reuses page
text, ordered link snapshots, and same-origin navigation. The model selects observed link
identifiers. Quickie resolves them to destinations, revalidates them against the live
document, and applies its navigation policy. A stale page or unsupported operation becomes
a tool result visible in the panel.

The LLM adapter selects Chat Completions or Responses at that base. It tries the alternate
route only when the endpoint rejects the route or its tool support. The first accepted
tool call fixes the format for that turn. The adapter sends each result with its original
call ID. It keeps continuation data in memory for the active turn and accepted history
within the workspace, matched to the same endpoint and format. Responses requests set
`store: false` and replay encrypted reasoning when the provider returns it.

The application owns the conversation's turn and tool state. The shell attaches the
workspace to its page views and renders status. The LLM adapter encodes provider tool
requests and accumulates responses. WebKit reports actual completion before the panel
records a successful action. Model output and page content have no direct path to the
automation protocol. Keep URL provenance, origin checks, and approval for irreversible
actions at the application and engine boundaries.

Additional WebKit input tools can enter the same tool set after each has a defined
permission check, a visible result, and conformance coverage for its actual execution.

## Acceptance

- A single entry accepts a question about a page and a request to open a visible link.
  The model's tool choice drives the outcome for each request.
- The workspace stays open for answer-only turns, navigation, failures, and follow-ups.
  Normal browser windows keep their content-first layout.
- A navigation row progresses from running to completed only after WebKit reports
  success. A stale document, error, or Stop updates that row and clears in-flight state.
- Navigation runs in the isolated profile. A model-generated URL or stale identifier
  cannot cause navigation in either profile.
- Opening the workspace and closing an idle workspace make zero LLM requests.
- Headless conformance exercises the fork against local fixtures. UI stories cover idle,
  running, cancellation, verified citations, and error states in both appearances.
