#!/usr/bin/env python3
"""Click through an interactive web visualization with Firefox WebDriver.

This avoids selenium/playwright dependencies by speaking the W3C WebDriver HTTP
protocol directly to geckodriver.
"""

from __future__ import annotations

import argparse
import base64
import json
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path


DEFAULT_CAPTURES = [
    ("step", "[data-demo-step]"),
    ("title", "[data-demo-title]"),
    ("caption", "[data-demo-caption]"),
    ("analysis_title", "[data-demo-analysis-title]"),
    ("analysis", "[data-demo-analysis]"),
    ("notice", "[data-demo-uip]"),
    ("trail", "[data-demo-trail]"),
    ("clauses", "[data-demo-clauses]"),
    ("cut_label", "[data-demo-cut-label]"),
]


def parse_size(value: str) -> tuple[int, int]:
    try:
        width, height = value.lower().split("x", 1)
        return int(width), int(height)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("size must look like 1440x1100") from exc


def parse_capture(value: str) -> tuple[str, str]:
    if "=" not in value:
        raise argparse.ArgumentTypeError("capture must look like name=css-selector")
    name, selector = value.split("=", 1)
    name = name.strip()
    selector = selector.strip()
    if not name or not selector:
        raise argparse.ArgumentTypeError("capture name and selector must be non-empty")
    return name, selector


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


class WebDriver:
    def __init__(self, geckodriver: str, port: int, log_path: Path):
        self.port = port
        self.base = f"http://127.0.0.1:{port}"
        self.log_file = log_path.open("w")
        self.proc = subprocess.Popen(
            [geckodriver, "--port", str(port)],
            stdout=self.log_file,
            stderr=subprocess.STDOUT,
        )
        self.session_id: str | None = None

    def close(self) -> None:
        if self.session_id:
            try:
                self.request("DELETE", f"/session/{self.session_id}", None, timeout=5)
            except Exception:
                pass
            self.session_id = None
        self.proc.terminate()
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log_file.close()

    def wait_until_ready(self) -> None:
        for _ in range(80):
            try:
                urllib.request.urlopen(f"{self.base}/status", timeout=0.2).read()
                return
            except Exception:
                time.sleep(0.1)
        raise RuntimeError("geckodriver did not become ready")

    def request(self, method: str, path: str, payload: object | None, timeout: int = 10) -> object:
        data = None if payload is None else json.dumps(payload).encode()
        request = urllib.request.Request(self.base + path, data=data, method=method)
        if payload is not None:
            request.add_header("Content-Type", "application/json")
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read().decode()
            return json.loads(body or "{}")

    def new_session(self, size: tuple[int, int]) -> None:
        width, height = size
        payload = {
            "capabilities": {
                "alwaysMatch": {
                    "browserName": "firefox",
                    "moz:firefoxOptions": {
                        "args": ["-headless", f"--width={width}", f"--height={height}"],
                    },
                },
            },
        }
        value = self.request("POST", "/session", payload, timeout=30)["value"]
        self.session_id = value["sessionId"]

    def end_session(self) -> None:
        if self.session_id:
            self.request("DELETE", f"/session/{self.session_id}", None, timeout=5)
            self.session_id = None

    def command(self, method: str, path: str, payload: object | None = None, timeout: int = 10) -> object:
        if not self.session_id:
            raise RuntimeError("no active session")
        return self.request(method, f"/session/{self.session_id}{path}", payload, timeout=timeout)[
            "value"
        ]

    def execute(self, script: str, args: list[object] | None = None, timeout: int = 10) -> object:
        return self.command("POST", "/execute/sync", {"script": script, "args": args or []}, timeout)

    def element_id(self, selector: str) -> str | None:
        try:
            value = self.command("POST", "/element", {"using": "css selector", "value": selector})
        except urllib.error.HTTPError:
            return None
        return value.get("element-6066-11e4-a52e-4f735466cecf")

    def click(self, selector: str) -> bool:
        element_id = self.element_id(selector)
        if not element_id:
            return False
        self.command("POST", f"/element/{element_id}/click", {})
        return True

    def screenshot(self, path: Path) -> None:
        encoded = self.command("GET", "/screenshot", timeout=20)
        path.write_bytes(base64.b64decode(encoded))


def collect_state(driver: WebDriver, root: str, selectors: dict[str, str], next_selector: str, back_selector: str, play_selector: str) -> dict:
    script = r"""
const rootSelector = arguments[0];
const captures = arguments[1];
const nextSelector = arguments[2];
const backSelector = arguments[3];
const playSelector = arguments[4];
const root = document.querySelector(rootSelector);
if (!root) return {error: `root not found: ${rootSelector}`};
const text = (selector) => root.querySelector(selector)?.innerText || "";
const disabled = (selector) => {
  const element = root.querySelector(selector);
  return element ? Boolean(element.disabled) : null;
};
const output = {
  captures: {},
  controls: {
    nextDisabled: disabled(nextSelector),
    backDisabled: disabled(backSelector),
    playDisabled: disabled(playSelector),
    playText: root.querySelector(playSelector)?.innerText || "",
  },
};
for (const [name, selector] of Object.entries(captures)) {
  output.captures[name] = text(selector);
}
return output;
"""
    return driver.execute(script, [root, selectors, next_selector, back_selector, play_selector])


def collect_overflow(driver: WebDriver) -> dict:
    script = r"""
const viewport = window.innerWidth;
const offenders = [];
for (const element of document.querySelectorAll("body *")) {
  const rect = element.getBoundingClientRect();
  if (rect.width > 0 && (rect.right > viewport + 2 || rect.left < -2)) {
    offenders.push({
      tag: element.tagName,
      id: element.id || "",
      className: typeof element.className === "string" ? element.className : "",
      text: (element.innerText || "").slice(0, 100),
      left: rect.left,
      right: rect.right,
      width: rect.width,
    });
  }
}
return {
  innerWidth: window.innerWidth,
  bodyWidth: document.body.getBoundingClientRect().width,
  scrollWidth: document.documentElement.scrollWidth,
  pageOverflows: document.documentElement.scrollWidth > document.body.getBoundingClientRect().width + 2,
  offenders: offenders.slice(0, 20),
};
"""
    return driver.execute(script)


def url_from_args(args: argparse.Namespace) -> str:
    if args.url:
        return args.url
    return Path(args.file).resolve().as_uri()


def run_desktop(driver: WebDriver, args: argparse.Namespace, selectors: dict[str, str], out: Path) -> dict:
    driver.new_session(args.desktop_size)
    try:
        driver.command("POST", "/url", {"url": url_from_args(args)}, timeout=30)
        time.sleep(args.load_wait)
        driver.execute(
            "document.querySelector(arguments[0])?.scrollIntoView({block:'start'});",
            [args.root],
        )
        time.sleep(0.25)

        screenshots = []
        states = []
        max_steps = args.steps if args.steps else args.max_steps
        for index in range(max_steps):
            state = collect_state(driver, args.root, selectors, args.next, args.back, args.play)
            state["index"] = index + 1
            states.append(state)
            screenshot_path = out / f"desktop-step-{index + 1}.png"
            driver.screenshot(screenshot_path)
            screenshots.append(str(screenshot_path))
            if state.get("controls", {}).get("nextDisabled") or index == max_steps - 1:
                break
            if not driver.click(args.next):
                break
            time.sleep(args.click_wait)

        control_checks = {}
        if driver.click(args.back):
            time.sleep(args.click_wait)
            control_checks["after_back"] = collect_state(driver, args.root, selectors, args.next, args.back, args.play)
        if driver.click(args.reset):
            time.sleep(args.click_wait)
            control_checks["after_reset"] = collect_state(driver, args.root, selectors, args.next, args.back, args.play)
        if args.play_wait > 0 and driver.click(args.play):
            time.sleep(args.play_wait)
            control_checks["after_play_wait"] = collect_state(
                driver, args.root, selectors, args.next, args.back, args.play
            )

        return {
            "viewport": {"width": args.desktop_size[0], "height": args.desktop_size[1]},
            "states": states,
            "control_checks": control_checks,
            "screenshots": screenshots,
        }
    finally:
        driver.end_session()


def run_mobile(driver: WebDriver, args: argparse.Namespace, selectors: dict[str, str], out: Path) -> dict:
    driver.new_session(args.mobile_size)
    try:
        driver.command("POST", "/url", {"url": url_from_args(args)}, timeout=30)
        time.sleep(args.load_wait)
        driver.execute(
            "document.querySelector(arguments[0])?.scrollIntoView({block:'start'});",
            [args.root],
        )
        time.sleep(0.25)
        clicks = max(0, args.mobile_step - 1)
        for _ in range(clicks):
            if not driver.click(args.next):
                break
            time.sleep(args.click_wait)

        state = collect_state(driver, args.root, selectors, args.next, args.back, args.play)
        overflow = collect_overflow(driver)
        screenshot_path = out / "mobile.png"
        driver.screenshot(screenshot_path)
        return {
            "viewport": {"width": args.mobile_size[0], "height": args.mobile_size[1]},
            "state": state,
            "overflow": overflow,
            "screenshot": str(screenshot_path),
        }
    finally:
        driver.end_session()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--file", help="Local HTML file to open")
    source.add_argument("--url", help="URL to open")
    parser.add_argument("--root", default="body", help="CSS selector for the visualization root")
    parser.add_argument("--next", default='[data-demo-action="next"]', help="CSS selector for Step/Next")
    parser.add_argument("--back", default='[data-demo-action="back"]', help="CSS selector for Back")
    parser.add_argument("--reset", default='[data-demo-action="reset"]', help="CSS selector for Reset")
    parser.add_argument("--play", default='[data-demo-action="play"]', help="CSS selector for Play")
    parser.add_argument("--steps", type=int, default=0, help="Expected number of steps; 0 means click until Next disables")
    parser.add_argument("--max-steps", type=int, default=20, help="Safety cap when --steps is 0")
    parser.add_argument("--capture", action="append", type=parse_capture, default=[], help="Add name=selector text capture")
    parser.add_argument("--out", default="/tmp/web-viz-walkthrough", help="Output directory")
    parser.add_argument("--desktop-size", type=parse_size, default=(1440, 1100))
    parser.add_argument("--mobile-size", type=parse_size, default=(430, 1200))
    parser.add_argument("--mobile-step", type=int, default=4)
    parser.add_argument("--no-mobile", action="store_true")
    parser.add_argument("--load-wait", type=float, default=1.0)
    parser.add_argument("--click-wait", type=float, default=0.25)
    parser.add_argument("--play-wait", type=float, default=0.0, help="Seconds to wait after clicking Play")
    parser.add_argument("--geckodriver", default="")
    args = parser.parse_args()

    geckodriver = args.geckodriver or shutil.which("geckodriver") or "/snap/bin/geckodriver"
    if not Path(geckodriver).exists():
        print(f"geckodriver not found: {geckodriver}", file=sys.stderr)
        return 2

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    selectors = dict(DEFAULT_CAPTURES)
    selectors.update(dict(args.capture))

    driver = WebDriver(geckodriver, free_port(), out / "geckodriver.log")
    try:
        driver.wait_until_ready()
        result = {
            "source": url_from_args(args),
            "root": args.root,
            "desktop": run_desktop(driver, args, selectors, out),
        }
        if not args.no_mobile:
            result["mobile"] = run_mobile(driver, args, selectors, out)
        (out / "states.json").write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps(result, indent=2))
    finally:
        driver.close()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
