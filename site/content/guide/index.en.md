+++
title = "Quickie guide"
description = "Use Quickie’s keyboard controls, Shields, agent, and developer tools."
+++

Quickie gives the whole window to the page. On macOS, open the address bar, tab switcher, or command palette when you need it. The shortcuts below are the defaults.

## Find your way around

| Shortcut | Action |
| --- | --- |
| `⌘L` | Open an address or search the web |
| `⌘⇧P` or `⌘K` | Search Quickie’s commands |
| `⌘P` | Find an open tab, bookmark, or page from history |
| `⌘F` | Find text on the current page |
| `Esc` | Dismiss the address bar or command palette |

## Work with tabs and links

- Press `⌘T` for a new tab or `⌘⇧N` for a throwaway tab. A throwaway tab keeps its site data separate and its visits out of your normal history.
- Tap `Ctrl+Tab` to return to the last-used tab. Keep the shortcut held to see thumbnails. Use `⌘W` to close a tab and `⌘⇧T` to reopen it.
- Press `⌘D` to bookmark the page. Use `⌘P` to find it again.
- Press `⌥F` and type a link’s hint to follow it. Start with `⇧⌥F` to open the link in a background tab.
- Open **Settings → Keyboard** to review or change shortcuts.

## Turn on Shields {#shields}

1. Open **Settings → Privacy & Shields** with `⌘,`.
2. Switch on the filter lists you want, or click **Enable** to fetch all offered lists. Use **Refresh Filter Lists** in the command palette to update them manually.
3. On a site, open Shields with `⌥⌘S`. If blocking interferes with the page, switch Shields off for that site and reload. The same panel lets you turn it back on.

Filter lists and scheduled list updates are off in a fresh profile. Enabling a list downloads it from its publisher.

## Update Quickie {#updates}

On macOS, select **Quickie → Check for Updates…** or open **Settings → General → Software Updates**. On Linux, open **Software Updates** with `Ctrl+,` or choose **Check for Updates** from Quickie’s launcher actions.

Quickie finds the matching package for your installation, verifies its SHA-256 and shows the release notes before a manual update. Turn on automatic updates if you want Quickie to download updates and install them when you quit. Your system may ask for permission to replace the installed app. **What’s New in Quickie** shows the notes for your current version offline.

If you are on 0.1.0, install 0.2.0 from GitHub Releases once to get the updater. [Download Quickie](https://github.com/izevg/quickie/releases).

If macOS blocks the first launch of an updated ad hoc build, approve it in **Privacy & Security → Open Anyway**. [Apple’s guidance](https://support.apple.com/en-us/102445).

## AI integration {#ai-integration}

Open **Settings → Agent** and configure an OpenAI-compatible endpoint and model. Use a local endpoint such as Ollama or LM Studio, or a hosted service you trust. Quickie chooses between Chat Completions and Responses. To ask about the current page, press `⌘P` and type `?` followed by a question. The Agent panel opens on the right and stays available for follow-up messages. Quickie sends the question and any page text the agent reads to the configured endpoint.

To open a link from the page, ask the agent to do it, for example `?open the first video`. The panel shows the tools it uses. Quickie checks the link against the live page and opens it in an isolated preview with fresh site data. Links and redirects stay within the original site origin. Use **Source page** to return to the browser page, **Stop** or `⌘.` to cancel a turn, and **Close Agent** to dismiss the panel.

For scripts, the macOS build with Quickie’s WebKit fork accepts `--automation-check` and an HTTP or HTTPS URL. It checks navigation in a temporary session and exits.

## For developers {#developers}

Use `F12` or `⌥⌘I` to show or hide Web Inspector and `⌥⌘J` to jump to Console. To open a specific tool, search the command palette for **Inspect Element**, **Network**, **Sources**, or **Performance**.

To call the isolated navigation check from a script, run this with the app installed in Applications:

```sh
/Applications/Quickie.app/Contents/MacOS/quickie --automation-check https://example.com/
```

Exit status zero means navigation completed. Use a local HTTP test page if the workflow must run offline. For the smaller Linux feature set and help with installation, see the [FAQ](../faq/).
