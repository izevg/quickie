"""Find real Web Inspector controls through the isolated desktop's AT-SPI bus."""

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi, GLib


DOCK_MENU = ("combo box", "Dock Web Inspector")
UNDOCK_BUTTON = ("button", "Detach into separate window")


def dock_control(story):
    Atspi.init()
    Atspi.set_timeout(400, 400)

    def find():
        return find_dock_control(Atspi.get_desktop(0))

    return story.wait(find, "the accessible Inspector dock control")


def find_dock_control(root):
    pending = [root]
    while pending:
        node = pending.pop()
        if node is None:
            continue
        try:
            control = (node.get_role_name(), node.get_name())
            if control == DOCK_MENU or control == UNDOCK_BUTTON:
                return node, control == DOCK_MENU
            pending.extend(node.get_child_at_index(i) for i in range(node.get_child_count()))
        except GLib.Error:
            # A WebKit accessibility subtree can be replaced while dock controls load.
            continue
    return None


def detach(story, control, menu):
    action = control.get_action_iface()
    assert action is not None and action.do_action(0), "The Inspector dock control did not activate"
    if menu:
        story.key("End", "Return")
