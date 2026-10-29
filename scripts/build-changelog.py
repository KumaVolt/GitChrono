#!/usr/bin/env python3
"""Builds site/changelog/index.html from the GitHub releases.

    python3 scripts/build-changelog.py              # fetch from the GitHub API
    python3 scripts/build-changelog.py --test       # run the converter's checks

Static HTML so search engines read every release. The Pages workflow runs it
before each deploy (GITHUB_TOKEN raises the API rate limit), and the release
workflow triggers that deploy, so a new release shows up on its own. It also
stamps the deploy with release data: softwareVersion in the homepage JSON-LD
and each sitemap <lastmod> (last commit to the page, or the newest release for
/changelog), so Google sees when pages really change.
"""
import datetime
import html
import json
import os
import pathlib
import re
import subprocess
import sys
import urllib.request

REPO = "KumaVolt/GitChrono"
ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "site" / "changelog" / "index.html"
SITE = "https://gitchrono.com/"
# Sections left out of the changelog because /download covers them.
SKIP_SECTIONS = {"install", "installation", "downloads"}


def inline(text):
    """Escapes text and renders `code`, **bold**, [links](url) and bare URLs."""
    parts = re.split(r"(`[^`]+`)", text)
    out = []
    for part in parts:
        if part.startswith("`") and part.endswith("`") and len(part) > 1:
            out.append("<code>" + html.escape(part[1:-1]) + "</code>")
            continue
        s = html.escape(part, quote=False)
        s = re.sub(r"\[([^\]]+)\]\((https?://[^)\s]+)\)", lambda m: f'<a href="{html.escape(m.group(2))}">{m.group(1)}</a>', s)
        s = re.sub(r'(?<![="\w>])(https?://[^\s<)]+)', lambda m: f'<a href="{m.group(1)}">{m.group(1)}</a>', s)
        s = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", s)
        out.append(s)
    return "".join(out)


def markdown(md):
    """The Markdown GitHub release notes use: headings, bold titles, lists,
    tables, paragraphs and [!NOTE]/[!WARNING] alerts."""
    out, para, items, table = [], [], [], []
    skipping = False

    def flush():
        if para:
            out.append("<p>" + inline(" ".join(para)) + "</p>")
            para.clear()
        if items:
            out.append("<ul>" + "".join(f"<li>{inline(i)}</li>" for i in items) + "</ul>")
            items.clear()
        if table:
            rows = [r for r in table if not re.fullmatch(r"\|?[\s:|-]+\|?", r)]
            cells = [[c.strip() for c in r.strip().strip("|").split("|")] for r in rows]
            if cells:
                head = "".join(f"<th>{inline(c)}</th>" for c in cells[0])
                body = "".join("<tr>" + "".join(f"<td>{inline(c)}</td>" for c in r) + "</tr>" for r in cells[1:])
                out.append(f"<table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table>")
            table.clear()

    lines = md.replace("\r\n", "\n").split("\n")
    i = 0
    while i < len(lines):
        line = lines[i].rstrip()
        heading = re.match(r"(#{1,6})\s+(.*)", line)
        if heading:
            flush()
            skipping = heading.group(2).strip().lower() in SKIP_SECTIONS
            if not skipping:
                tag = "h3" if len(heading.group(1)) <= 2 else "h4"
                out.append(f"<{tag}>{inline(heading.group(2).strip())}</{tag}>")
            i += 1
            continue
        if skipping:
            i += 1
            continue
        if line.startswith(">"):
            flush()
            quote = []
            while i < len(lines) and lines[i].startswith(">"):
                quote.append(lines[i][1:].strip())
                i += 1
            alert = re.fullmatch(r"\[!(\w+)\]", quote[0]) if quote else None
            text = " ".join(q for q in (quote[1:] if alert else quote) if q)
            out.append(f'<div class="callout">{inline(text)}</div>')
            continue
        if not line.strip():
            flush()
        elif line.lstrip().startswith("|"):
            if para or items:
                flush()
            table.append(line)
        elif re.match(r"\s*[-*]\s+", line):
            if para or table:
                flush()
            items.append(re.sub(r"^\s*[-*]\s+", "", line))
        elif items and line.startswith("  "):
            items[-1] += " " + line.strip()
        elif re.fullmatch(r"\*\*[^*]+\*\*", line.strip()):
            flush()
            out.append(f"<h4>{inline(line.strip()[2:-2])}</h4>")
        else:
            if items or table:
                flush()
            para.append(line.strip())
        i += 1
    flush()
    return "\n".join(out)


def fetch_releases():
    req = urllib.request.Request(f"https://api.github.com/repos/{REPO}/releases?per_page=100", headers={"Accept": "application/vnd.github+json", "User-Agent": "gitchrono-site"})
    if os.environ.get("GITHUB_TOKEN"):
        req.add_header("Authorization", "Bearer " + os.environ["GITHUB_TOKEN"])
    with urllib.request.urlopen(req, timeout=30) as r:
        return [rel for rel in json.load(r) if not rel.get("draft")]


PLATFORM = [("macos-universal.dmg", "macOS"), ("windows-x86_64.zip", "Windows"), ("linux-x86_64.tar.gz", "Linux x86_64"), ("linux-aarch64.tar.gz", "Linux ARM64")]


def release_html(rel, latest):
    tag = rel["tag_name"]
    published = datetime.datetime.fromisoformat(rel["published_at"].replace("Z", "+00:00"))
    files = []
    for suffix, label in PLATFORM:
        asset = next((a for a in rel.get("assets", []) if a["name"].endswith(suffix)), None)
        if asset:
            files.append(f'<a href="{html.escape(asset["browser_download_url"])}">↓ {label}</a>')
    notes = markdown(rel.get("body") or "") or "<p>Maintenance release.</p>"
    badge = '<span class="latest">Latest</span>' if latest else ""
    pre = '<span class="latest" style="color:var(--amber);border-color:rgba(227,179,65,.45)">Pre-release</span>' if rel.get("prerelease") else ""
    return f"""      <article class="release" id="{html.escape(tag)}">
        <div class="when">
          <h2><a href="#{html.escape(tag)}">{html.escape(tag)}</a></h2>
          <time datetime="{published.date().isoformat()}">{published.strftime("%B %-d, %Y")}</time>
          {badge}{pre}
          <div class="files">{"".join(files)}<a href="{html.escape(rel["html_url"])}">On GitHub →</a></div>
        </div>
        <div class="prose">
{notes}
        </div>
      </article>"""


def page(releases):
    latest_tag = next((r["tag_name"] for r in releases if not r.get("prerelease")), None)
    body = "\n".join(release_html(r, r["tag_name"] == latest_tag) for r in releases) or '      <p class="sub">The first release is on its way.</p>'
    template = (ROOT / "scripts" / "changelog.template.html").read_text()
    return template.replace("{{RELEASES}}", body).replace("{{UPDATED}}", datetime.date.today().isoformat())


def stamp_sitemap(xml, dates):
    """Sets each <url>'s <lastmod> to dates[loc], where given."""
    def one(m):
        date = dates.get(m.group(2))
        return m.group(0) if not date else re.sub(r"<lastmod>[^<]*</lastmod>", f"<lastmod>{date}</lastmod>", m.group(0))
    return re.sub(r"<url>(\s*<loc>([^<]+)</loc>.*?)</url>", one, xml, flags=re.S)


def last_commit_date(*paths):
    """YYYY-MM-DD of the last commit touching paths ("" without git history)."""
    try:
        return subprocess.run(["git", "log", "-1", "--format=%cs", "--", *paths], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    except OSError:
        return ""


def stamp(releases):
    stable = [r for r in releases if not r.get("prerelease")]
    index = ROOT / "site" / "index.html"
    if stable:
        version = stable[0]["tag_name"].lstrip("v")
        index.write_text(re.sub(r'"softwareVersion": "[^"]*"', f'"softwareVersion": "{version}"', index.read_text()))
    sitemap = ROOT / "site" / "sitemap.xml"
    dates = {}
    for loc in re.findall(r"<loc>([^<]+)</loc>", sitemap.read_text()):
        page = "site/" + loc.removeprefix(SITE) + "index.html"
        dates[loc] = last_commit_date(page)
    newest = max((r["published_at"][:10] for r in releases), default="")
    dates[SITE + "changelog/"] = max(newest, last_commit_date("scripts/changelog.template.html", "scripts/build-changelog.py"))
    sitemap.write_text(stamp_sitemap(sitemap.read_text(), {k: v for k, v in dates.items() if v}))


def test():
    md = "> [!WARNING]\n> **Alpha.** Keep backups.\n\n## What's new\n\n**Graph**\n- Date order, like `git log`\n  and GitKraken\n- See [docs](https://x.y/z)\n\n## Install\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n**Full Changelog**: https://github.com/a/b/compare/v1...v2"
    h = markdown(md)
    assert '<div class="callout"><strong>Alpha.</strong> Keep backups.</div>' in h, h
    assert "<h3>What&#x27;s new</h3>" in h or "<h3>What's new</h3>" in h, h
    assert "<h4>Graph</h4>" in h and "<li>Date order, like <code>git log</code> and GitKraken</li>" in h, h
    assert '<a href="https://x.y/z">docs</a>' in h, h
    assert "<table>" not in h and "Install" not in h, "install section skipped"
    assert '<strong>Full Changelog</strong>: <a href="https://github.com/a/b/compare/v1...v2">' not in h, "follows the skipped section"
    assert "<code>&lt;b&gt;</code>" in inline("`<b>`") and "&lt;script&gt;" in inline("<script>")
    t = markdown("| A | B |\n|---|---|\n| 1 | `x` |")
    assert t == "<table><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td>1</td><td><code>x</code></td></tr></tbody></table>", t
    xml = "<urlset>\n  <url>\n    <loc>https://a/</loc>\n    <lastmod>2020-01-01</lastmod>\n  </url>\n  <url>\n    <loc>https://a/b/</loc>\n    <lastmod>2020-01-01</lastmod>\n  </url>\n</urlset>"
    x = stamp_sitemap(xml, {"https://a/b/": "2026-10-27"})
    assert x == xml.replace("<lastmod>2020-01-01</lastmod>\n  </url>\n</urlset>", "<lastmod>2026-10-27</lastmod>\n  </url>\n</urlset>"), x
    print("ok")


if __name__ == "__main__":
    if "--test" in sys.argv:
        test()
    else:
        OUT.parent.mkdir(parents=True, exist_ok=True)
        releases = fetch_releases()
        OUT.write_text(page(releases))
        stamp(releases)
        print(f"wrote {OUT.relative_to(ROOT)} ({len(releases)} releases)")
