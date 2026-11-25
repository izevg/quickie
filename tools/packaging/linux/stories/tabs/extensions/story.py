"""Check extension restoration and profile isolation using the installed browser."""

from contextlib import closing
import json
from pathlib import Path
import sqlite3

from ..library.story import private_navigation
from ..story import painted


FIXTURE = "tabs/extensions/fixture.html"
SITE = "http://127.0.0.1/*"
OTHER_SITE = "http://localhost/*"
PAGE_COLOR = (116, 68, 98)


def save_packages(story):
    profile = Path(story.environment["XDG_DATA_HOME"]) / "quickie"
    packages = (
        ("a" * 32, "allowed", True, SITE),
        ("b" * 32, "disabled", False, SITE),
        ("c" * 32, "ungranted", True, OTHER_SITE),
    )
    with closing(sqlite3.connect(profile / "shell.sqlite")) as database, database:
        for position, (extension_id, marker, enabled, grant) in enumerate(packages):
            directory = profile / "extensions" / marker
            directory.mkdir(parents=True)
            (directory / "manifest.json").write_text(json.dumps({
                "manifest_version": 3,
                "name": f"Restore fixture {marker}",
                "version": "1.0",
                "host_permissions": [SITE],
                "content_scripts": [{
                    "matches": [SITE], "js": ["mark.js"], "run_at": "document_start",
                }],
            }), encoding="utf-8")
            (directory / "mark.js").write_text(
                'document.addEventListener("DOMContentLoaded", () => {'
                f"document.documentElement.dataset[{json.dumps(marker)}] = '1';"
                '}, {once: true});',
                encoding="utf-8",
            )
            database.execute(
                "INSERT INTO extensions (position, id, name, version, source, location, enabled) "
                "VALUES (?, ?, ?, '1.0', 'folder', ?, ?)",
                (position, extension_id, marker, str(directory), enabled),
            )
            database.execute(
                "INSERT INTO extension_grants (extension, kind, name) VALUES (?, 'pattern', ?)",
                (extension_id, grant),
            )


def get_markers(story, phase):
    def observed():
        for markers in range(8):
            if story.requests(f"extension-result?phase={phase}&markers={markers}"):
                return (bool(markers & 1), bool(markers & 2), bool(markers & 4))
        return None

    return story.wait(observed, f"extension markers for the {phase} document")


def run(story):
    # Let the normal browser write its schema, then emulate packages already approved by the user.
    story.close(story.open())
    save_packages(story)

    window = story.open(f"{FIXTURE}?ordinary")
    ordinary = get_markers(story, "ordinary")
    story.save(painted(story, window, PAGE_COLOR), "extensions-restored")

    private_navigation(story, window, f"{FIXTURE}?private", PAGE_COLOR)
    private = get_markers(story, "private")
    story.capture(window, "extensions-private")
    assert ordinary == (True, False, False), (
        f"Ordinary browsing must run only the enabled and granted script. Observed {ordinary}."
    )
    assert private == (False, False, False), (
        f"Private browsing must run zero ordinary-profile scripts. Observed {private}."
    )
