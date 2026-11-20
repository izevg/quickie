// Reads what the command stored, so the popup shows that storage works.
chrome.storage.local.get("count").then(({ count }) => {
  if (count) {
    document.getElementById("count").textContent = `The command ran ${count} time(s).`;
  }
});
