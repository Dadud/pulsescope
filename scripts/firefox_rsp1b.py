"""Connect RSP1B and start a real FM Broadcast scan, then screenshot."""
import asyncio
from playwright.async_api import async_playwright

URL = "http://127.0.0.1:8765/?token=firefox-test"
OUT_DIR = r"D:\pulsescope\screenshots"
CONSOLE: list[str] = []

async def main() -> None:
    async with async_playwright() as p:
        browser = await p.firefox.launch(headless=True, args=["--window-size=1280,800"])
        ctx = await browser.new_context(viewport={"width": 1280, "height": 800})
        page = await ctx.new_page()
        page.on("console", lambda msg: CONSOLE.append(f"console.{msg.type}: {msg.text}"))
        page.on("pageerror", lambda exc: CONSOLE.append(f"pageerror: {exc}"))
        page.on("websocket", lambda ws: CONSOLE.append(f"ws_open: {ws.url}"))

        await page.goto(URL, wait_until="networkidle", timeout=20000)
        await page.wait_for_timeout(3000)  # give SvelteKit time to mount and open WS

        # Verify auth header is being sent by checking browser dev tools
        auth_check = await page.evaluate("() => localStorage.getItem('pst')")
        CONSOLE.append(f"auth_token_in_storage: {auth_check}")
        loc_check = await page.evaluate("() => ({ href: window.location.href, search: window.location.search, hostname: window.location.hostname, port: window.location.port, proto: window.location.protocol })")
        CONSOLE.append(f"location: {loc_check}")
        # Inspect active WS
        ws_count = await page.evaluate("() => performance.getEntries().filter(e => e.name.includes('127.0.0.1:8765')).length")
        CONSOLE.append(f"perf_ws_count: {ws_count}")
        # Check whether any 401/200 came back
        responses = await page.evaluate("() => performance.getEntriesByType('resource').filter(r => r.name.includes('127.0.0.1:8765') && !r.name.includes('_app/')).map(r => ({ name: r.name.slice(0, 100), status: r.responseStatus })).slice(0, 20)")
        CONSOLE.append(f"api_responses: {responses}")

        # Connect RSP1B and start scan via the API (the UI's buttons are
        # state-coupled and harder to drive; the API is the canonical path).
        await page.request.fetch("http://127.0.0.1:8765/device/connect", method="POST", headers={"content-type": "application/json", "authorization": "Bearer firefox-test"}, data='{"key":"driver=sdrplay"}')
        await page.wait_for_timeout(1500)
        await page.request.fetch("http://127.0.0.1:8765/scan/start", method="POST", headers={"content-type": "application/json", "authorization": "Bearer firefox-test"}, data='{"range_name":"FM Broadcast"}')
        await page.wait_for_timeout(8000)  # let frames accumulate

        await page.screenshot(path=f"{OUT_DIR}/03-rsp1b-running.png", full_page=True)

        # Pull runtime state via the page's own request context (carries
        # cookies but not our auth-token query; we'll add the header).
        async def fetch_json(path):
            r = await page.request.fetch(f"http://127.0.0.1:8765{path}", headers={"authorization": "Bearer firefox-test"})
            return await r.json()
        stats = await fetch_json("/debug/stats")
        signal_events = await fetch_json("/signal_events?limit=5")
        vfo_states = await fetch_json("/vfo/states")
        scan_ranges = await fetch_json("/channels/banks")

        with open(f"{OUT_DIR}/runtime.txt", "w", encoding="utf-8") as f:
            f.write(f"stats: {stats}\n")
            f.write(f"signal_events (top 5): {signal_events}\n")
            f.write(f"vfo_states: {vfo_states}\n")
            f.write(f"scan_ranges (count): {len(scan_ranges) if isinstance(scan_ranges, list) else 'n/a'}\n")

        print(f"frames_processed={stats.get('frames_processed')}")
        print(f"audio callback_frames={stats.get('audio', {}).get('callback_frames')}")
        print(f"audio output_peak={stats.get('audio', {}).get('output_peak')}")
        print(f"audio underruns={stats.get('audio', {}).get('underrun_samples')}")
        print(f"signal_events_count={len(signal_events)}")
        print(f"vfo_count={len(vfo_states)}")
        if signal_events: print(f"first_hit: {signal_events[0]}")
        print("---")
        print(f"CONSOLE_LINES={len(CONSOLE)}")
        for line in CONSOLE[-30:]:
            print(" ", line)

        # Screenshot the signal history
        await page.screenshot(path=f"{OUT_DIR}/04-final.png", full_page=True)

        await ctx.close()
        await browser.close()

asyncio.run(main())
