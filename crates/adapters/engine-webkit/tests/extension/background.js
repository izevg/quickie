// The command counts its runs in storage and tells the active tab's content script.
chrome.commands.onCommand.addListener(async (command) => {
  if (command !== "mark-page") {
    return;
  }
  const { count = 0 } = await chrome.storage.local.get("count");
  await chrome.storage.local.set({ count: count + 1 });
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (tab && tab.id !== undefined) {
    chrome.tabs.sendMessage(tab.id, { mark: true, count: count + 1 });
  }
});
