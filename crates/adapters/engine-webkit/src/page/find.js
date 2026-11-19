// Counts the matches of a find, marks them, and tells which one WebKit's own find selected.
// The body of an async function run by `callAsyncJavaScript` in Quickie's isolated content
// world, so the page can neither see nor replace it. Arguments: `query` (string), `found`
// (whether WebKit's find matched). Returns "current/total", current counted from 1, 0 when
// unknown. No network, nothing stored beyond the page's lifetime.
//
// It counts what WebKit's find searches: rendered text of the main frame, case-insensitive,
// a run of spaces in the query matching any run of white space. Text in subframes, shadow
// roots, and form fields is not counted.

const ALL = 'quickie-find';
const CURRENT = 'quickie-find-current';
const MARKED = 1000;

const highlights = globalThis.CSS && CSS.highlights;
if (highlights) {
  highlights.delete(ALL);
  highlights.delete(CURRENT);
}
const words = query.split(/\s+/).filter(Boolean);
if (!found || words.length === 0) {
  return '0/0';
}

const state = (globalThis.quickieFind ??= {});
if (!state.sheet && 'adoptedStyleSheets' in document) {
  state.sheet = new CSSStyleSheet();
  state.sheet.replaceSync(
    `::highlight(${ALL}) { background-color: rgb(255 214 10 / 0.4); }` +
      `::highlight(${CURRENT}) { background-color: rgb(255 159 10); color: black; }`,
  );
}
if (state.sheet && !document.adoptedStyleSheets.includes(state.sheet)) {
  document.adoptedStyleSheets = [...document.adoptedStyleSheets, state.sheet];
}

const shown = new Map();
const isShown = (element) => {
  if (!element) {
    return false;
  }
  let visible = shown.get(element);
  if (visible === undefined) {
    visible = element.checkVisibility
      ? element.checkVisibility({ visibilityProperty: true })
      : element.getClientRects().length > 0;
    shown.set(element, visible);
  }
  return visible;
};

const nodes = [];
const starts = [];
let text = '';
const root = document.body ?? document.documentElement;
const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
for (let node = walker.nextNode(); node; node = walker.nextNode()) {
  if (node.data && isShown(node.parentElement)) {
    nodes.push(node);
    starts.push(text.length);
    text += node.data;
  }
}

const escaped = words.map((word) => word.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
const matches = [];
for (const match of text.matchAll(new RegExp(escaped.join('\\s+'), 'giu'))) {
  matches.push([match.index, match.index + match[0].length]);
}

// The text node holding offset `at`: for an end, the one it ends in.
const locate = (at, end) => {
  let low = 0;
  let high = starts.length - 1;
  while (low < high) {
    const middle = (low + high + 1) >> 1;
    if (end ? starts[middle] < at : starts[middle] <= at) {
      low = middle;
    } else {
      high = middle - 1;
    }
  }
  return [nodes[low], at - starts[low]];
};
const rangeOf = ([from, to]) => {
  const range = new Range();
  range.setStart(...locate(from, false));
  range.setEnd(...locate(to, true));
  return range;
};

let current = 0;
const selection = getSelection();
if (selection && selection.rangeCount > 0) {
  const selected = selection.getRangeAt(0);
  const index = nodes.indexOf(selected.startContainer);
  if (index >= 0) {
    const at = starts[index] + selected.startOffset;
    current = matches.findIndex(([from]) => from === at) + 1;
  }
}

if (highlights && typeof Highlight === 'function' && matches.length > 0) {
  highlights.set(ALL, new Highlight(...matches.slice(0, MARKED).map(rangeOf)));
  if (current > 0) {
    const mark = new Highlight(rangeOf(matches[current - 1]));
    mark.priority = 1;
    highlights.set(CURRENT, mark);
  }
}
return `${current}/${matches.length}`;
