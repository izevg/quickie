"""Process lifecycle and screenshot decoding for isolated GTK story sessions."""

from io import BytesIO
import subprocess
import time

from PIL import Image


def stop(process):
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=4)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def start(stack, *args, **options):
    process = subprocess.Popen(args, **options)
    stack.callback(stop, process)
    return process


def wait(probe, label, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            result = probe()
        except subprocess.CalledProcessError:
            result = None
        if result:
            return result
        time.sleep(0.12)
    raise RuntimeError(f"Timed out waiting for {label}")


def decode(png):
    with Image.open(BytesIO(png)) as image:
        return image.convert("RGB")
