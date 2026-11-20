// Shows that the content script ran, and marks the page when the background asks.
const banner = document.createElement("div");
banner.id = "quickie-test-extension";
banner.textContent = "Test extension: content script ran";
banner.style.cssText =
  "position:fixed;left:16px;bottom:16px;padding:8px 12px;border-radius:8px;" +
  "font:13px -apple-system,sans-serif;background:#2b6cb0;color:white;z-index:2147483647";
(document.body || document.documentElement).appendChild(banner);

chrome.runtime.onMessage.addListener((message) => {
  if (message && message.mark) {
    banner.textContent = `Test extension: command ran (${message.count})`;
    banner.style.background = "#2f855a";
  }
});
