"""Captures d'écran de Lugus Transfer en mode démo, dans les trois langues.

    pip install playwright pillow && playwright install chromium
    python docs/tools/captures.py          # fr en es
    python docs/tools/annoter.py           # cadres et pastilles dans docs/guide/, images du README

Les captures brutes et la position des éléments sont écrites dans docs/tools/out/.
L'horloge de la page est figée : les captures sont identiques d'une fois sur l'autre.
"""
import asyncio, json, os, pathlib, sys
from playwright.async_api import async_playwright

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "docs" / "tools" / "out"
URL = (ROOT / "ui" / "index.html").as_uri()
CHROME = os.environ.get("CHROME")  # chemin de Chrome, sinon Chromium de Playwright
SCALE = 1.5
LINK = "lugus://r/ng6bfcmwftaipe1e6x8tpdnhp5ijxqcixciyurigwfs3hg9ng58o?v=1"
CODE = "4821-lanterne-7"

SEL = {
    "modes": ".modes", "drop": "#drop", "picks": ".picks", "after": "#sendForm > div:last-child",
    "go": "#go", "stop": "#stop", "lang": "#lang", "about": "#about", "verBadge": "#verBadge", "settings": "#settings",
    "optUpdate": "#modalBox .opt", "chip": "#chip", "lid": "#lidWarn",
    "clear": "#clearSource", "status": "#status", "steps": "#steps", "phase": "#phase",
    "picker": "#picker", "tools": "#pickTools", "tree": "#tree", "sum": "#pickSum", "pickOk": "#pickOk",
    "share": "#share", "shareLink": "#shareLink", "pub": "#pub", "shareCode": "#shareCode",
    "copyRow": "#copyLink", "codeRow": "#share .row:nth-of-type(2)", "newCode": "#newCode", "tip": "#share .valid",
    "prog": "#prog", "pct": "#prog .pct", "bar": "#prog > .track", "vtrack": "#prog .vtrack", "metrics": "#prog .metrics",
    "log": "#log", "files": "#files", "banner": "#banner", "bannerBtns": "#banner .row",
    "link": "#link", "code": "#code", "dest": "#recvForm .pick", "night": "#night",
    "modal": "#modalBox", "modalOk": "#modalOk", "warnbox": "#modalBox .warnbox",
}

async def shot(page, name):
    await page.wait_for_timeout(400)
    await page.screenshot(path=str(OUT / f"{name}.png"))
    boxes = {}
    for key, query in SEL.items():
        b = await page.evaluate(
            "q=>{const e=document.querySelector(q);if(!e)return null;const r=e.getBoundingClientRect();"
            "if(!r.width||!r.height)return null;return [r.left,r.top,r.right,r.bottom]}", query)
        if b:
            boxes[key] = [round(v * SCALE) for v in b]
    json.dump(boxes, open(OUT / f"{name}.json", "w"))
    print("  ", name)

async def fresh(browser, lang, accepted=True, newer=None):
    ctx = await browser.new_context(viewport={"width": 1280, "height": 820}, device_scale_factor=SCALE, locale=lang)
    page = await ctx.new_page()
    # Recherche de nouvelle version : jamais de vraie requête. Sans `newer`, on
    # fait comme si l'ordinateur était hors ligne (le badge montre la version).
    async def releases(route):
        if newer:
            await route.fulfill(status=200, content_type="application/json",
                                headers={"access-control-allow-origin": "*"},
                                body=json.dumps([{"tag_name": "v" + newer, "draft": False}]))
        else:
            await route.abort()
    await page.route("https://api.github.com/**", releases)
    await page.clock.install(time="2026-11-15T14:32:00")
    init = "window.LUGUS_DEMO_RATE=110e6;"  # 110 Mo/s, une fibre à 1 Gbit/s
    init += f"window.LUGUS_DEMO_CODE='{CODE}';"
    init += f"localStorage.setItem('lugus.lang',JSON.stringify('{lang}'));localStorage.setItem('lugus.mode',JSON.stringify('send'));"
    if accepted:
        init += "localStorage.setItem('lugus.accepted',JSON.stringify(true));"
    await page.add_init_script(init)
    await page.goto(URL)
    await page.add_style_tag(content=".demo{display:none!important}")
    await page.clock.run_for(300)
    return page

async def run(page, ms):
    await page.clock.run_for(ms)

async def until(page, js, step=100, limit=4_000_000):
    waited = 0
    while waited < limit:
        if await page.evaluate(js):
            return
        await run(page, step); waited += step
    raise RuntimeError("délai dépassé : " + js)

async def main(lang):
    async with async_playwright() as p:
        b = await (p.chromium.launch(executable_path=CHROME) if CHROME else p.chromium.launch())

        # --- Envoi
        page = await fresh(b, lang, accepted=False)
        await shot(page, f"welcome_{lang}")
        await page.click("#modalOk"); await run(page, 200)
        await shot(page, f"send_empty_{lang}")

        await page.click("#pickFolder"); await run(page, 300)
        await page.click('.row[data-dir="Proxy/"] .nm'); await run(page, 100)
        await page.click('.row[data-dir="A002/"] input'); await run(page, 100)
        await page.evaluate("document.activeElement.blur()")
        await shot(page, f"send_choose_{lang}")

        await page.click("#go"); await run(page, 900)
        await shot(page, f"send_publishing_{lang}")
        await until(page, "document.querySelector('#pub').classList.contains('online')")
        await run(page, 300)
        await shot(page, f"send_link_{lang}")

        await until(page, "S.stepIdx >= S.steps.indexOf('transfer') && parseInt(document.querySelector('#pct').textContent) >= 48", step=5000)
        await shot(page, f"send_progress_{lang}")
        await until(page, "S.phase && S.phase.key === 'ph_remote_verify' && parseInt(document.querySelector('#vpct').textContent) >= 85", step=2000)
        await shot(page, f"send_verify_{lang}")
        await until(page, "!document.querySelector('#banner').classList.contains('hidden')", step=2000)
        await run(page, 300)
        await shot(page, f"send_done_{lang}")
        await page.context.close()

        # --- Réception
        page = await fresh(b, lang)
        await page.click("#modeRecv"); await run(page, 100)
        await page.fill("#link", LINK); await page.fill("#code", CODE)
        await page.click("#pickDest"); await run(page, 600)
        await page.evaluate("document.activeElement.blur()")
        await shot(page, f"recv_form_{lang}")

        await page.click("#go")
        await until(page, "!document.querySelector('#picker').classList.contains('hidden')")
        await run(page, 200)
        await page.click('.tool[data-kind="Original"]'); await run(page, 100)
        await page.evaluate("document.activeElement.blur()")
        await shot(page, f"recv_choose_{lang}")

        await page.click("#pickOk")
        await until(page, "S.phase && S.phase.key === 'ph_receiving' && parseInt(document.querySelector('#pct').textContent) >= 55", step=1000)
        await shot(page, f"recv_progress_{lang}")
        await until(page, "S.phase && S.phase.key === 'ph_local_verify' && parseInt(document.querySelector('#vpct').textContent) >= 75", step=150)
        await shot(page, f"recv_verify_{lang}")
        await until(page, "S.phase && S.phase.key === 'ph_mhl'", step=20)
        await shot(page, f"recv_mhl_{lang}")
        await until(page, "!document.querySelector('#banner').classList.contains('hidden')")
        await run(page, 300)
        await shot(page, f"recv_done_{lang}")
        await page.context.close()

        # --- Version et réglages
        page = await fresh(b, lang, newer="0.3.0")
        await run(page, 500)
        await shot(page, f"update_{lang}")
        await page.click("#settings"); await run(page, 300)
        await shot(page, f"settings_{lang}")
        await page.context.close()

        # --- Mode Noctiluca
        page = await fresh(b, lang)
        await page.click("#modeRecv"); await run(page, 100)
        await page.fill("#link", LINK); await page.fill("#code", CODE)
        await page.click("#pickDest"); await run(page, 600)
        await page.check("#nightOn"); await page.fill("#startAt", "01:00")
        await page.select_option("#recvAfter", "sleep")
        await page.evaluate("document.activeElement.blur()"); await run(page, 300)
        await page.click("#go"); await run(page, 500)
        await shot(page, f"recv_night_{lang}")
        await page.context.close()
        await b.close()

if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    for lang in (sys.argv[1:] or ["fr", "en", "es"]):
        print(lang)
        asyncio.run(main(lang))
