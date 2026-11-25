// The command counts its runs in storage and tells the active tab's content script.
let status = "waiting for command";

chrome.commands.onCommand.addListener(async (command) => {
  if (command !== "mark-page") {
    return;
  }
  try {
    status = "command received, reading storage";
    const { count = 0 } = await chrome.storage.local.get("count");
    await chrome.storage.local.set({ count: count + 1 });
    const { count: saved } = await chrome.storage.local.get("count");
    status = "storage updated, querying active tab";
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (!tab || tab.id === undefined) {
      status = "no active tab in the current window";
      return;
    }
    status = `sending marker to tab ${tab.id}`;
    await chrome.tabs.sendMessage(tab.id, { mark: true, count: saved });
    status = `marker delivered to tab ${tab.id}`;
  } catch (error) {
    status = `command failed: ${error.message}`;
  }
});

chrome.runtime.onMessage.addListener((message, _sender, respond) => {
  if (message && message.status) {
    respond({ status });
  }
});
