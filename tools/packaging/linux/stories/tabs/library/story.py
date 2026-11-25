"""Exercise native navigation, durable library sources, and throwaway history isolation."""

from contextlib import closing
from pathlib import Path
import sqlite3

from ..hints.story import label_at
from ..keyboard import control_tab
from ..story import address_visible, changed, painted, text_updated

FIXTURE = "tabs/library/fixture.html"
ARTICLE = f"{FIXTURE}?page=article"
BACKGROUND = f"{FIXTURE}?page=background"
NOTEBOOK = f"{FIXTURE}?page=notebook"
TITLE_EVENT = f"{FIXTURE}?page=title-event"
BLUE = (52, 85, 119)
TEAL = (61, 111, 112)
MAUVE = (144, 74, 100)
TITLE = "Library alpha manuscript"
RETITLED = "Library alpha second edition"
ARCHIVE = "Library bronze archive"
PRIVATE = "Library private notebook"


def rows(story, query):
    profile = Path(story.environment["XDG_DATA_HOME"]) / "quickie" / "shell.sqlite"
    with closing(sqlite3.connect(f"{profile.as_uri()}?mode=ro", uri=True)) as database:
        return database.execute(query).fetchall()


def history(story):
    return {url: (title, count) for url, title, count in rows(
        story, "SELECT url, title, visit_count FROM history",
    )}


def bookmarks(story):
    return dict(rows(story, "SELECT url, title FROM bookmarks"))


def visit(story, path, title, count):
    address = story.url(path)
    story.wait(lambda: history(story).get(address) == (title, count), f"{title} with {count} visits")


def from_address(story, window, text):
    address = address_visible(story, window)
    story.paste(text)
    return text_updated(story, window, address, 20)


def private_navigation(story, window, path, color=MAUVE):
    story.key("ctrl+shift+n")
    from_address(story, window, story.url(path))
    story.key("Return")
    painted(story, window, color)


def run(story):
    window = story.open(ARTICLE)
    idle = painted(story, window, BLUE)
    visit(story, ARTICLE, TITLE, 1)

    story.key("u")
    visit(story, ARTICLE, RETITLED, 1)
    story.key("ctrl+d", "ctrl+d")
    story.wait(lambda: bookmarks(story) == {story.url(ARTICLE): RETITLED}, "one saved bookmark")

    story.key("shift+alt+f")
    story.wait(
        lambda: label_at(story.scale, story.picture(window), idle, 60, 92),
        "the native label for the background article",
    )
    story.key("a")
    visit(story, BACKGROUND, ARCHIVE, 1)
    painted(story, window, BLUE)
    with control_tab(story, hold_ms=325):
        pass
    painted(story, window, TEAL)
    assert story.requests(BACKGROUND) == 1, "switching reloaded the background article"
    story.key("ctrl+w")
    painted(story, window, BLUE)

    story.key("ctrl+r")
    visit(story, ARTICLE, TITLE, 2)

    requests = story.requests(ARTICLE)
    private_navigation(story, window, ARTICLE, BLUE)
    story.wait(
        lambda: story.requests(ARTICLE) > requests,
        "the exact typed address loads in the throwaway tab instead of switching to the normal tab",
    )
    story.key("ctrl+w")
    painted(story, window, BLUE)

    private_navigation(story, window, f"{ARTICLE}#private")
    before_title = story.requests(TITLE_EVENT)
    story.key("u")
    story.wait(lambda: story.requests(TITLE_EVENT) > before_title, "the private title update")
    story.key("ctrl+w")
    painted(story, window, BLUE)

    private_navigation(story, window, NOTEBOOK)
    assert bookmarks(story) == {story.url(ARTICLE): RETITLED}, "a private visit saved a bookmark"
    story.key("ctrl+d")
    saved = {story.url(ARTICLE): RETITLED, story.url(NOTEBOOK): PRIVATE}
    story.wait(lambda: bookmarks(story) == saved, "the explicitly requested private bookmark")
    story.key("ctrl+w")
    painted(story, window, BLUE)

    story.key("ctrl+w")
    story.close(window)
    before_reopen = {story.url(ARTICLE): (TITLE, 2), story.url(BACKGROUND): (ARCHIVE, 1)}
    assert history(story) == before_reopen, "extra visits or a private title survived shutdown"
    assert bookmarks(story) == saved, "bookmarks did not preserve their saved titles"

    window = story.open()
    idle = story.picture(window)
    story.key("ctrl+p")
    opened = changed(story, window, idle, "the Go To palette")
    story.paste(RETITLED)
    story.save(text_updated(story, window, opened, 15), "library-bookmark-go-to")
    story.key("Return")
    painted(story, window, BLUE)
    visit(story, ARTICLE, TITLE, 3)

    story.key("ctrl+l")
    results = from_address(story, window, ARCHIVE)
    story.key("Down")
    story.save(changed(story, window, results, "the history row selected"), "library-history-open")
    story.key("Return")
    painted(story, window, TEAL)
    visit(story, BACKGROUND, ARCHIVE, 2)

    story.key("alt+Left")
    painted(story, window, BLUE)
    visit(story, ARTICLE, TITLE, 4)
    story.key("alt+Right")
    painted(story, window, TEAL)
    visit(story, BACKGROUND, ARCHIVE, 3)
    story.close(window)
    assert history(story) == {story.url(ARTICLE): (TITLE, 4), story.url(BACKGROUND): (ARCHIVE, 3)}
    assert bookmarks(story) == saved
