"""Advent of Code HTTP client: fetch puzzle/input and submit answers."""

from __future__ import annotations

import re
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from enum import Enum
from html.parser import HTMLParser
from typing import Optional


AOC_ORIGIN = "https://adventofcode.com"
DEFAULT_USER_AGENT = (
    "Agent-Kuibysheff aoc-live-workflow "
    "(+https://github.com/gybson63/Agent-Kuibysheff; educational example)"
)


class SubmitVerdict(str, Enum):
    CORRECT = "correct"
    WRONG = "wrong"
    TOO_RECENT = "too_recent"
    ALREADY_SOLVED = "already_solved"
    WRONG_LEVEL = "wrong_level"
    AUTH_REQUIRED = "auth_required"
    UNKNOWN = "unknown"


@dataclass(frozen=True)
class PuzzlePage:
    year: int
    day: int
    title: str
    url: str
    text: str
    raw_html: str


@dataclass(frozen=True)
class SubmitResult:
    verdict: SubmitVerdict
    message: str
    raw_html: str
    wait_seconds: Optional[int] = None
    hint: Optional[str] = None  # e.g. "too high" / "too low"


class _ArticleCollector(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.articles: list[str] = []
        self.title = ""
        self._in_article = False
        self._depth = 0
        self._chunks: list[str] = []
        self._in_h2 = False
        self._h2_chunks: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, Optional[str]]]) -> None:
        attr = {k: (v or "") for k, v in attrs}
        if tag == "article" and "day-desc" in attr.get("class", "").split():
            self._in_article = True
            self._depth = 1
            self._chunks = []
            return
        if self._in_article:
            if tag == "article":
                self._depth += 1
            if tag in {"p", "li", "pre", "h2", "code"}:
                self._chunks.append("\n")
            if tag == "h2":
                self._in_h2 = True
                self._h2_chunks = []
            if tag == "br":
                self._chunks.append("\n")

    def handle_endtag(self, tag: str) -> None:
        if self._in_article and tag == "h2":
            self._in_h2 = False
            title = "".join(self._h2_chunks).strip()
            if title and not self.title:
                self.title = title
            self._chunks.append("\n")
        if not self._in_article:
            return
        if tag == "article":
            self._depth -= 1
            if self._depth <= 0:
                text = re.sub(r"\n{3,}", "\n\n", "".join(self._chunks)).strip()
                if text:
                    self.articles.append(text)
                self._in_article = False
                self._depth = 0
                self._chunks = []

    def handle_data(self, data: str) -> None:
        if self._in_h2:
            self._h2_chunks.append(data)
        if self._in_article:
            self._chunks.append(data)


class AocHttpClient:
    def __init__(self, session_cookie: str, user_agent: str = DEFAULT_USER_AGENT) -> None:
        token = session_cookie.strip()
        if not token:
            raise ValueError("AOC_SESSION cookie is empty")
        self._session = token
        self._user_agent = user_agent

    def _request(
        self,
        method: str,
        path: str,
        *,
        data: Optional[bytes] = None,
        content_type: Optional[str] = None,
    ) -> tuple[int, str]:
        url = f"{AOC_ORIGIN}{path}"
        headers = {
            "Cookie": f"session={self._session}",
            "User-Agent": self._user_agent,
        }
        if content_type:
            headers["Content-Type"] = content_type
        req = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=60) as resp:
                body = resp.read().decode("utf-8", errors="replace")
                return int(resp.status), body
        except urllib.error.HTTPError as err:
            body = err.read().decode("utf-8", errors="replace")
            return int(err.code), body

    def fetch_puzzle(self, year: int, day: int, part: int) -> PuzzlePage:
        if part not in (1, 2):
            raise ValueError("part must be 1 or 2")
        status, html = self._request("GET", f"/{year}/day/{day}")
        if status == 404:
            raise RuntimeError(f"AoC puzzle not found: {year}-day-{day}")
        if status != 200:
            raise RuntimeError(f"AoC puzzle fetch failed HTTP {status}")
        if "To play, please identify yourself" in html:
            raise RuntimeError("AoC session cookie rejected (identify yourself)")

        parser = _ArticleCollector()
        parser.feed(html)
        if not parser.articles:
            raise RuntimeError("Could not parse day-desc article from AoC HTML")

        # Part 1 = first article; part 2 prefers the second when unlocked.
        if part == 2 and len(parser.articles) >= 2:
            text = parser.articles[1]
        else:
            text = parser.articles[0]
            if part == 2 and len(parser.articles) < 2:
                text = (
                    text
                    + "\n\nNOTE: Part 2 description is not unlocked yet on the page "
                    "(complete part 1 first). Solve part 1 if that is the intent."
                )

        title = parser.title or f"Day {day}"
        url = f"{AOC_ORIGIN}/{year}/day/{day}"
        return PuzzlePage(
            year=year,
            day=day,
            title=title,
            url=url,
            text=text,
            raw_html=html,
        )

    def fetch_input(self, year: int, day: int) -> str:
        status, body = self._request("GET", f"/{year}/day/{day}/input")
        if status == 404:
            raise RuntimeError(f"AoC input not found: {year}-day-{day}")
        if status != 200:
            raise RuntimeError(f"AoC input fetch failed HTTP {status}")
        if "Please don't repeatedly request this endpoint" in body:
            raise RuntimeError("AoC rate-limited input downloads")
        if "To play, please identify yourself" in body:
            raise RuntimeError("AoC session cookie rejected while fetching input")
        return body if body.endswith("\n") else body + "\n"

    def submit_answer(self, year: int, day: int, part: int, answer: str) -> SubmitResult:
        form = urllib.parse.urlencode(
            {"level": str(part), "answer": answer}
        ).encode("utf-8")
        status, html = self._request(
            "POST",
            f"/{year}/day/{day}/answer",
            data=form,
            content_type="application/x-www-form-urlencoded",
        )
        if status != 200:
            return SubmitResult(
                verdict=SubmitVerdict.UNKNOWN,
                message=f"HTTP {status} from AoC submit",
                raw_html=html,
            )
        return classify_submit_html(html)


def classify_submit_html(html: str) -> SubmitResult:
    article = _extract_main_article(html) or html
    plain = re.sub(r"<[^>]+>", " ", article)
    plain = re.sub(r"\s+", " ", plain).strip()

    if "To play, please identify yourself" in plain:
        return SubmitResult(SubmitVerdict.AUTH_REQUIRED, plain, html)

    if "That's the right answer" in plain:
        return SubmitResult(SubmitVerdict.CORRECT, plain, html)

    if "Did you already complete it" in plain:
        return SubmitResult(SubmitVerdict.ALREADY_SOLVED, plain, html)

    if "You don't seem to be solving the right level" in plain:
        return SubmitResult(SubmitVerdict.WRONG_LEVEL, plain, html)

    if "You gave an answer too recently" in plain:
        wait = _parse_wait_seconds(plain)
        return SubmitResult(SubmitVerdict.TOO_RECENT, plain, html, wait_seconds=wait)

    if "That's not the right answer" in plain:
        hint = None
        lower = plain.lower()
        if "too high" in lower:
            hint = "too high"
        elif "too low" in lower:
            hint = "too low"
        wait = _parse_wait_seconds(plain)
        return SubmitResult(
            SubmitVerdict.WRONG, plain, html, wait_seconds=wait, hint=hint
        )

    return SubmitResult(SubmitVerdict.UNKNOWN, plain or "(empty response)", html)


def _extract_main_article(html: str) -> Optional[str]:
    match = re.search(
        r"<article[^>]*>(.*?)</article>", html, flags=re.IGNORECASE | re.DOTALL
    )
    return match.group(1) if match else None


def _parse_wait_seconds(text: str) -> Optional[int]:
    # Examples: "please wait 1 minute", "please wait 5 minutes", "wait 30 seconds"
    m = re.search(
        r"(?:please\s+)?wait\s+(\d+)\s+(minute|minutes|second|seconds)",
        text,
        flags=re.IGNORECASE,
    )
    if not m:
        return None
    n = int(m.group(1))
    unit = m.group(2).lower()
    if unit.startswith("minute"):
        return n * 60
    return n
