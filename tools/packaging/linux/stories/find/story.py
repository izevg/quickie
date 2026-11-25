"""Drive the real GTK find bar and WebKit text highlights."""

from PIL import ImageChops


def run(story):
    window = story.open("find/fixture.html")
    scale = story.scale

    def page_ready():
        image = story.picture(window)
        marker = (image.width - 18 * scale, image.height - 18 * scale)
        return image if image.getpixel(marker) == (95, 226, 176) else None

    idle = story.wait(page_ready, "WebKit renders searchable passages")

    def passages(image):
        return tuple(
            image.crop((60 * scale, top * scale, 650 * scale, (top + 50) * scale)).tobytes()
            for top in range(150, 450, 50)
        )

    def controls(image):
        return image.crop((image.width - 410 * scale, 12 * scale, image.width, 65 * scale))

    def counter(image):
        return image.crop((image.width - 205 * scale, 29 * scale,
                           image.width - 123 * scale, 49 * scale)).tobytes()

    base = passages(idle)

    def changed_rows(before, after):
        return tuple(index for index, (left, right) in enumerate(zip(before, after)) if left != right)

    def bar_visible():
        return ImageChops.difference(controls(idle), controls(story.picture(window))).getbbox()

    def highlighted(rows, previous_count):
        def probe():
            image = story.picture(window)
            return image if (
                changed_rows(base, passages(image)) == rows and counter(image) != previous_count
            ) else None
        return story.wait(probe, "WebKit highlights the matching passages and reports their count")

    def query(text):
        story.wait(bar_visible, "the Find entry is available")
        story.key("ctrl+a", "BackSpace")
        story.wait(lambda: counter(story.picture(window)) == blank, "Find clears its previous query")
        story.paste(text)

    def moved(previous, rows):
        def changed():
            image = story.picture(window)
            return image if changed_rows(passages(previous), passages(image)) == rows else None
        image = story.wait(changed, f"selection moves across exact rows {rows}")
        assert counter(image) == counter(first), "Stepping retains the verified total on an unchanged page"
        return image

    story.key("ctrl+f")
    story.wait(bar_visible, "Ctrl+F opens the find bar")
    blank = counter(story.picture(window))
    query("sovereign")
    first = highlighted((0, 1, 2, 3), blank)

    story.key("shift+Return")
    last = moved(first, (0, 3))
    story.save(last, "steps-backward-wrap")
    story.key("Return")
    wrapped = moved(last, (0, 3))
    assert passages(wrapped) == passages(first), "Enter must wrap to the first match"
    story.save(wrapped, "steps")

    story.key("ctrl+g")
    second = moved(wrapped, (0, 1))
    story.key("Return")
    third = moved(second, (1, 2))
    story.save(third, "matches")
    story.key("ctrl+shift+g")
    returned = moved(third, (1, 2))
    assert passages(returned) == passages(second), "Ctrl+Shift+G selects the prior occurrence"

    query("sovereignty")
    no_matches = highlighted((), blank)
    assert bar_visible(), "A query with no matches keeps Find in Page open"
    assert counter(no_matches) != counter(second), "The counter must show the no-match state"
    story.save(no_matches, "no-matches")

    query("суверен")
    cyrillic = highlighted((4, 5), blank)
    assert counter(cyrillic) != counter(no_matches), "Cyrillic search must report its matches"
    story.save(cyrillic, "cyrillic")

    story.key("Escape")
    story.wait(lambda: not bar_visible(), "Escape dismisses the find bar")
    # WebKit retains the active match as ordinary page selection after clearing its markers.
    def no_find_markers():
        current = passages(story.picture(window))
        return all(base[index] == current[index] for index in (0, 1, 2, 3, 5))

    story.wait(no_find_markers, "Escape clears WebKit's other find highlights")
    story.key("F8")

    def page_focused():
        image = story.picture(window)
        marker = (image.width - 18 * scale, image.height - 18 * scale)
        return image.getpixel(marker) == (109, 130, 238)

    story.wait(page_focused, "Escape restores WebKit page focus")

    story.key("ctrl+f")
    query("sovereign")
    highlighted((0, 1, 2, 3), blank)
    story.key("Escape")
    story.wait(lambda: not bar_visible(), "Find closes before an independent page selection")
    story.key("F9")
    story.wait(
        lambda: changed_rows(base, passages(story.picture(window))) == (2,),
        "the third match has a normal page selection",
    )
    story.key("ctrl+f")
    reopened = highlighted((0, 1, 2, 3), blank)
    assert counter(reopened) == counter(first), "The total is independent of the page selection"
    story.save(reopened, "find-from-page-selection")

    story.click(window, 670 * scale, 130 * scale)
    story.key("F6")

    def fragment_navigated():
        image = story.picture(window)
        marker = (image.width - 18 * scale, image.height - 18 * scale)
        return image if image.getpixel(marker) == (226, 169, 95) else None

    after_fragment = story.wait(fragment_navigated, "the page performs fragment navigation")
    assert counter(after_fragment) == counter(reopened), "A same-document change retains Find"

    story.key("F7")

    def added_match():
        image = story.picture(window)
        return image if changed_rows(base, passages(image)) == (0, 1, 2, 3, 5) else None

    before_refresh = story.wait(added_match, "the fixture adds a later match in the document")
    story.key("ctrl+g")

    def refreshed_match():
        image = fragment_navigated()
        if image is None:
            return None
        current = passages(image)
        return image if (
            changed_rows(base, current) == (0, 1, 2, 3, 5)
            and current[5] != passages(before_refresh)[5]
            and counter(image) not in (blank, counter(reopened))
        ) else None

    refreshed = story.wait(refreshed_match, "the new hit gets a marker and the total updates")
    story.save(refreshed, "dynamic-matches")

    path = "find/fixture.html?next-document"
    story.key("F5")
    story.wait(lambda: story.requests(path), "navigation requests a new document")
    story.wait(page_ready, "the new document renders")
    story.key("ctrl+f")
    fresh = highlighted((0, 1, 2, 3), blank)
    assert counter(fresh) == counter(first), "Find reopens with a fresh total on the new document"
