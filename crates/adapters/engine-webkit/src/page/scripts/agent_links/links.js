// Runs in Quickie's isolated content world. Only the current main document is read.
(() => {
    const maxLinks = 100;
    const maxAnchors = 1024;
    const maxURLLength = 2048;
    const maxLabelLength = 160;
    const documentURL = new URL(location.href);
    if (documentURL.href.length > maxURLLength)
        return '';
    const visible = new Map();
    const below = new Map();
    const ancestorDepths = new WeakMap();
    const maxDepth = 64;

    function excluded(element) {
        return element.hidden || element.inert || element.getAttribute('aria-hidden') === 'true';
    }

    // Only this synchronous scan shares the cache. A null depth means this caller ran out of
    // budget, so leave that ancestor uncached for calls starting nearer the root.
    function ancestorDepth(ancestor, remaining) {
        if (!ancestor)
            return 0;
        const known = ancestorDepths.get(ancestor);
        if (known !== undefined)
            return known;
        if (!remaining)
            return null;
        const parent = excluded(ancestor) ? -1 : ancestorDepth(ancestor.parentElement, remaining - 1);
        if (parent === null)
            return null;
        const depth = parent < 0 ? -1 : parent + 1;
        ancestorDepths.set(ancestor, depth);
        return depth;
    }

    function rendered(link) {
        if (!link.checkVisibility({checkOpacity: true, checkVisibilityCSS: true}) || excluded(link))
            return false;
        const depth = ancestorDepth(link.parentElement, maxDepth - 1);
        return depth !== null && depth >= 0 && depth < maxDepth;
    }

    function compactLabel(value) {
        if (!value)
            return '';
        const text = value.slice(0, 512).trim().replace(/\s+/g, ' ');
        if (text.length <= maxLabelLength)
            return text.toWellFormed();
        return Array.from(text).slice(0, maxLabelLength).join('').toWellFormed();
    }

    function labelFor(link) {
        return compactLabel(link.getAttribute('aria-label')) || compactLabel(link.title)
            || compactLabel(link.innerText) || compactLabel(link.querySelector('img[alt]')?.alt);
    }

    if (/^https?:$/.test(documentURL.protocol) && !documentURL.username && !documentURL.password) {
        const width = innerWidth;
        const height = innerHeight;
        const anchors = document.links;
        for (let index = 0; index < maxAnchors && visible.size < maxLinks; ++index) {
            const link = anchors.item(index);
            if (!link)
                break;
            if (link.localName !== 'a' || link.hasAttribute('download'))
                continue;
            const raw = link.getAttribute('href')?.trim();
            if (!raw || raw.startsWith('#') || raw.length > maxURLLength)
                continue;
            let url;
            try {
                url = new URL(raw, link.baseURI);
            } catch {
                continue;
            }
            if (url.protocol !== documentURL.protocol || url.origin !== documentURL.origin
                || url.username || url.password || url.href.length > maxURLLength || url.href === documentURL.href
                || visible.has(url.href) || !rendered(link))
                continue;
            const rect = link.getBoundingClientRect();
            if (rect.width <= 0 || rect.height <= 0)
                continue;
            const intersectsViewport = rect.left < width && rect.right > 0 && rect.top < height && rect.bottom > 0;
            const group = intersectsViewport ? visible : below;
            if (group.size === maxLinks || group.has(url.href))
                continue;
            const label = labelFor(link);
            if (label)
                group.set(url.href, {url: url.href, label});
        }
    }
    const offscreen = [...below.values()].filter(({url}) => !visible.has(url));
    const links = [...visible.values(), ...offscreen].slice(0, maxLinks);
    return JSON.stringify({document: documentURL.href, links});
})()
