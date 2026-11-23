+++
title = "Quickie FAQ"
description = "System requirements, macOS installation, Linux preview, and privacy."
+++

## What Macs can run Quickie?

Quickie 0.2.0 targets **Apple silicon** Macs running **macOS 26.5 or later**. Check [GitHub Releases](https://github.com/izevg/quickie/releases) for available builds and their requirements.

## How do I install it on macOS?

Download the DMG and its SHA-256 checksum from the same Quickie release. From the download directory, run `shasum -a 256 -c filename.dmg.sha256` using the actual checksum filename. If it reports **OK**, open the DMG and drag Quickie to Applications.

If the release notes identify an ad hoc build and macOS blocks the first launch because it cannot verify the developer, try opening Quickie once. After verifying the download, you can approve this copy in **System Settings → Privacy & Security → Open Anyway**. See [Apple’s guidance on opening apps](https://support.apple.com/en-us/102445).

## What does the Linux preview include?

The first Linux preview targets **x86-64** and uses GTK4 with Quickie’s WebKitGTK fork. Choose `.deb` for Ubuntu 24.04 LTS, `.rpm` for Fedora 43, or AppImage to run Quickie without installing a package. [Find packages and checksums on GitHub Releases](https://github.com/izevg/quickie/releases).

Use `Ctrl+L` to open the address bar, `Ctrl+R` to reload, `Alt+Left` and `Alt+Right` for history, and `Ctrl+Shift+I` for Web Inspector. The preview has one browsing window. Session restore, Shields, the built-in agent, and the macOS Inspector styling are outside its scope.

## What does Quickie send over the network?

Quickie sends no telemetry. Sites you open still make their own requests. You choose whether Quickie downloads filter lists or refreshes them on a schedule. Ask sends your question and page text to the configured model. Act sends your request and labels from a limited list of links. If the endpoint is hosted, its operator handles those requests.

## Why am I still seeing ads?

Filter lists are off in a fresh profile. Enable the lists in **Settings → Privacy & Shields**, then check that Shields is on for the site and reload. The Shields section of the [guide](../guide/) also explains how to make exceptions for individual sites.

## Where can I report a problem?

Open a [GitHub issue](https://github.com/izevg/quickie/issues) with the Quickie version, your operating system, and the steps that reproduce the issue. The [guide](../guide/) covers the main controls.
