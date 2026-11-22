(() => {
    const find = (selector) => document.querySelector(selector);
    const all = (selector) => Array.from(document.querySelectorAll(selector));
    const rect = (node) => node?.getBoundingClientRect();
    const visible = (node) => {
        const bounds = rect(node);
        return bounds && bounds.width > 0 && bounds.height > 0 && getComputedStyle(node).visibility === "visible";
    };
    const tab = (label) => all(".tab-bar [role=tab]").find((item) => item.textContent.trim() === label);
    const selected = (label) => tab(label)?.getAttribute("aria-selected") === "true";
    const message = (level, text) => all(`.console-messages .console-${level}-level`).find((row) => row.textContent.includes(text));
    const warning = () => message("warning", "Inspector story warning");
    const error = () => message("error", "Inspector story error");
    const warningLink = () => warning()?.querySelector(".console-message-location .source-link, .console-message-location a");
    const network = () => find(".tab.network .network-table");
    const request = () => all(".tab.network .table .cell.name").find((cell) => cell.title.endsWith("/inspector/story.js"));
    const table = () => network()?.querySelector(":scope > .table");
    const detail = () => network()?.querySelector(":scope > .network-detail");
    const back = () => find("button[aria-label='Back to requests']");

    // These controls handle mousedown in Web Inspector instead of click activation alone.
    const press = (element) => {
        if (!visible(element))
            return "the control is not visible";
        const bounds = rect(element);
        const options = {bubbles: true, cancelable: true, view: window, clientX: bounds.x + bounds.width / 2, clientY: bounds.y + bounds.height / 2};
        element.dispatchEvent(new MouseEvent("mousedown", {...options, buttons: 1}));
        element.dispatchEvent(new MouseEvent("mouseup", options));
        element.click();
        return true;
    };

    return {
        consoleSelected() {
            return selected("Console") || "the Console tab is not selected";
        },

        watchConsoleErrors() {
            const errors = window.inspectorStoryErrors = [];
            window.addEventListener("error", (event) => errors.push(event.message));
            window.addEventListener("unhandledrejection", (event) => errors.push(String(event.reason)));
            return true;
        },

        consoleLayout() {
            const bar = find(".tab-bar");
            const toolbar = find(".quickie-console-toolbar");
            const clear = toolbar?.querySelector(".clear-log");
            const filter = toolbar?.querySelector("input[aria-label='Filter Console output']");
            const log = find(".tab.console .console-messages");
            const promptBand = find(".quick-console");
            const prompt = promptBand?.querySelector(".console-prompt");
            if (![bar, toolbar, clear, filter, log, promptBand, prompt].every(visible))
                return "Console tab, toolbar, filter, log, or prompt is not yet visible";

            const nativeInset = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--quickie-native-controls-left"));
            const firstTab = all(".tab-bar [role=tab]").find(visible);
            const dock = find("select[aria-label='Dock Web Inspector']");
            if (!(nativeInset > 20) || rect(firstTab).left < nativeInset - 1 || rect(bar).top > 2 || !visible(dock))
                return "the tab strip does not reserve space beside the real window controls";

            if (rect(clear).right > rect(filter).left + 2 || rect(toolbar).bottom > rect(log).top + 2 || rect(log).bottom > rect(promptBand).top + 2 || rect(promptBand).height < 40)
                return `unexpected layout: Clear right ${rect(clear).right}, filter left ${rect(filter).left}, toolbar bottom ${rect(toolbar).bottom}, log ${rect(log).top}–${rect(log).bottom}, prompt band top ${rect(promptBand).top} height ${rect(promptBand).height}`;

            return true;
        },

        consoleEntries() {
            if (!visible(warning()) || !visible(error())) {
                const scope = all(".log-scope-bar .selected").map((scope) => scope.textContent.trim());
                const pending = WI.consoleLogViewController?._pendingMessagesForSession?.size;
                return `missing live rows; visibility=${document.visibilityState}, focus=${document.hasFocus()}, pending=${pending}, scope=${scope.join(",")}, errors=${window.inspectorStoryErrors.join(" | ")}`;
            }
            return true;
        },

        consoleSourceVisible() {
            const log = find(".tab.console .console-messages");
            if (!visible(warningLink()))
                return "the warning's source control is missing";
            if (!(rect(warning()).top < rect(error()).top) || rect(warningLink()).right > rect(log).right + 1)
                return "the live entries or the source link do not fit the log";
            return true;
        },

        openSource() {
            const link = warningLink();
            if (!visible(link))
                return "the warning source link is not visible";
            link.click();
            return true;
        },

        sourceLanded() {
            return (selected("Sources") && all(".tab.sources .CodeMirror-code").some((source) => visible(source) && source.textContent.includes("Inspector story warning")))
                || "the warning link did not navigate to the fetched script in Sources";
        },

        networkTabAvailable() {
            return visible(tab("Network")) || "the Network tab is not visible";
        },

        selectNetwork() {
            return press(tab("Network"));
        },

        networkRequestVisible() {
            return (selected("Network") && visible(request())) || "the live request did not appear in the selected Network tab";
        },

        openRequest() {
            return press(request());
        },

        wideNetwork() {
            const toolbar = find(".tab.network .network-toolbar");
            const urlFilter = toolbar?.querySelector(".filter-bar input");
            const filters = toolbar?.querySelector(".network-type-filter-scope-bar");
            const clear = toolbar?.querySelector(".clear-network-items");
            const preserve = toolbar?.querySelector(".network-preserve-requests");
            const status = table()?.querySelector(".header .cell.status");
            const summary = detail()?.querySelector(".request-summary");
            const headers = detail()?.querySelector(".resource-headers");
            if (![table(), detail(), urlFilter, filters, clear, preserve, status, summary, headers].every(visible))
                return "wide Network is missing list columns, inline controls, or request headers";
            if (rect(detail()).left < rect(table()).left + 399 || rect(status).right > rect(detail()).left + 1)
                return "the side-by-side details overlap the request list";
            if (!summary.textContent.includes("story.js") || !summary.textContent.includes("200"))
                return "the request summary does not show the real response";
            return true;
        },

        stackedNetwork() {
            const divider = find("[role=separator][aria-label='Request details height']");
            const filters = find(".tab.network .network-filter-toolbar .network-type-filter-scope-bar");
            if (![table(), detail(), divider, filters].every(visible))
                return `request table, details, filters, or separator is missing at ${innerWidth}×${innerHeight}`;
            if (rect(table()).bottom > rect(divider).bottom + 2 || rect(divider).bottom > rect(detail()).top + 2 || Math.abs(rect(table()).left - rect(detail()).left) > 2)
                return "the request details no longer stack underneath the table";
            return true;
        },

        drawerNetwork() {
            return (visible(detail()) && visible(back()) && table()?.inert && !visible(table()) && back().parentElement.textContent.includes("story.js"))
                || `the compact drawer did not expose the selected request and hide the other requests at ${innerWidth}×${innerHeight}`;
        },

        backToRequests() {
            if (!visible(back()))
                return "Back to requests is not visible";
            back().click();
            return true;
        },

        requestsReturned() {
            return (visible(request()) && !table().inert && !visible(back()) && !visible(detail()) && table().contains(document.activeElement))
                || "Back to requests did not restore the list and keyboard focus";
        },
    };
})()
