"""Drive Firefox (via Playwright) at the PulseScope server, capture screenshots
and key DOM state, then exit."""
import asyncio
from playwright.async_api import async_playwright

URL = "http://127.0.0.1:8765/?token=firefox-test"
OUT_DIR = r"D:\pulsescope\screenshots"

async def main() -> None:
    import os
    os.makedirs(OUT_DIR, exist_ok=True)
    async with async_playwright() as p:
        browser = await p.firefox.launch(headless=True, args=["--window-size=1280,800"])
        ctx = await browser.new_context(viewport={"width": 1280, "height": 800})
        page = await ctx.new_page()
        console = []
        page.on("console", lambda msg: console.append(f"{msg.type}: {msg.text}"))
        page.on("pageerror", lambda exc: console.append(f"pageerror: {exc}"))

        # SvelteKit routing may not push state to hash; we use ?token query so
        # the API client picks it up from window.location.
        await page.goto(URL, wait_until="networkidle", timeout=20000)
        await page.wait_for_timeout(2000)  # give SvelteKit a moment to mount

        await page.screenshot(path=f"{OUT_DIR}/01-initial.png", full_page=True)

        # Pull some DOM signal
        title = await page.title()
        body_html = await page.evaluate("() => document.body.innerText")
        vfo_count = await page.evaluate("() => document.querySelectorAll('.vfo-tile').length")
        range_count = await page.evaluate("() => document.querySelectorAll('.range-row').length")
        canvas_count = await page.evaluate("() => document.querySelectorAll('canvas').length")

        # Try connecting a mock device and starting the scan
        try:
            await page.evaluate("() => fetch('/device/connect', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ key: 'driver=mock' }) })")
            await page.wait_for_timeout(1000)
            await page.evaluate("() => fetch('/scan/start', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ range_name: 'FM Broadcast' }) })")
            await page.wait_for_timeout(4000)  # let a few spectrum frames render
        except Exception as e:
            console.append(f"scan control error: {e}")

        await page.screenshot(path=f"{OUT_DIR}/02-after-scan.png", full_page=True)

        # Trigger a hardware test on a real RSP1B if available
        try:
            rsp = await page.evaluate("""() => fetch('/device/connect', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ key: 'driver=sdrplay' }) }).then(r => r.json())""")
            console.append(f"rsp1b_connect: {rsp}")
        except Exception as e:
            console.append(f"rsp1b error: {e}")

        # Inspect status of UI after scan
        status_pill = await page.evaluate("""() => {
          const el = document.querySelector('.device-strip');
          return el ? el.innerText : 'no device-strip';
        }""")
        ws_active = await page.evaluate("() => !!window.WebSocket && window.WebSocket.OPEN")

        # Squelch slider on the first VFO if any
        vfo_squelch = await page.evaluate("""() => {
          const el = document.querySelector('.vfo-tile .signal-dot');
          return el ? getComputedStyle(el).backgroundColor : null;
        }""")

        with open(f"{OUT_DIR}/dom.txt", "w", encoding="utf-8") as f:
            f.write(f"title: {title}\n")
            f.write(f"vfo_count: {vfo_count}\n")
            f.write(f"range_count: {range_count}\n")
            f.write(f"canvas_count: {canvas_count}\n")
            f.write(f"status_pill: {status_pill}\n")
            f.write(f"ws_active: {ws_active}\n")
            f.write(f"vfo_squelch_color: {vfo_squelch}\n")
            f.write(f"body_text_first_500: {body_html[:500]}\n")
            f.write(f"console_lines: {len(console)}\n")
            for line in console[-50:]:
                f.write(f"  {line}\n")

        print("OK title=", title)
        print(f"vfo_count={vfo_count} range_count={range_count} canvas_count={canvas_count}")
        print(f"status_pill={status_pill!r} ws_active={ws_active}")
        print(f"vfo_squelch_color={vfo_squelch}")
        print(f"console_lines={len(console)}")
        for line in console[-20:]:
            print(" ", line)

        await ctx.close()
        await browser.close()

asyncio.run(main())
