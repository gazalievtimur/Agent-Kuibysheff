"""ACP stdio client bridge for a long-lived agent_Kuibyshev process."""

from __future__ import annotations

import asyncio
import contextlib
import logging
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional

from acp import PROTOCOL_VERSION, spawn_agent_process, text_block
from acp.interfaces import Client
from acp.schema import AgentMessageChunk, AgentThoughtChunk

logger = logging.getLogger(__name__)


@dataclass
class PromptOutcome:
    stop_reason: str
    answer: str
    messages: list[str] = field(default_factory=list)
    thoughts: list[str] = field(default_factory=list)


class _CollectingClient(Client):
    """ACP client that accumulates agent message chunks for answer extraction."""

    def __init__(self) -> None:
        self.messages: list[str] = []
        self.thoughts: list[str] = []

    async def session_update(self, session_id: str, update: Any, **kwargs: Any) -> None:
        del session_id, kwargs
        text = _content_text(update)
        if text is None:
            return
        if isinstance(update, AgentMessageChunk) or _update_kind(update) == "agent_message_chunk":
            self.messages.append(text)
        elif isinstance(update, AgentThoughtChunk) or _update_kind(update) == "agent_thought_chunk":
            self.thoughts.append(text)

    async def request_permission(
        self, session_id: Any, tool_call: Any, options: Any, **kwargs: Any
    ) -> dict[str, Any]:
        del session_id, tool_call, options, kwargs
        # Kuibyshev does not solicit ACP permissions for tools; refuse if asked.
        return {"outcome": {"outcome": "cancelled"}}


def _update_kind(update: Any) -> str:
    kind = getattr(update, "session_update", None)
    if kind is None and isinstance(update, dict):
        kind = update.get("sessionUpdate") or update.get("session_update")
    return str(kind or "")


def _content_text(update: Any) -> Optional[str]:
    content = getattr(update, "content", None)
    if content is None and isinstance(update, dict):
        content = update.get("content")
    if content is None:
        return None
    text = getattr(content, "text", None)
    if text is None and isinstance(content, dict):
        text = content.get("text")
    if text is None:
        return None
    return str(text)


class AcpAgentSession:
    """Owns one long-lived `agent_Kuibyshev acp` child and one ACP session."""

    def __init__(
        self,
        *,
        agent_bin: Path,
        config: Path,
        settings_dir: Path,
        home: Path,
        cwd: Path,
        save_chat_history: bool = True,
    ) -> None:
        self.agent_bin = agent_bin
        self.config = config
        self.settings_dir = settings_dir
        self.home = home
        self.cwd = cwd
        self.save_chat_history = save_chat_history
        self._client = _CollectingClient()
        self._cm: Any = None
        self._conn: Any = None
        self._proc: Any = None
        self._session_id: Optional[str] = None
        self._stderr_task: Optional[asyncio.Task[None]] = None

    async def __aenter__(self) -> "AcpAgentSession":
        args = [
            "acp",
            "--config",
            str(self.config),
            "--settings-dir",
            str(self.settings_dir),
            "--home",
            str(self.home),
        ]
        if self.save_chat_history:
            args.append("--save-chat-history")

        self._cm = spawn_agent_process(
            self._client,
            str(self.agent_bin),
            *args,
            cwd=str(self.cwd),
        )
        self._conn, self._proc = await self._cm.__aenter__()
        self._stderr_task = asyncio.create_task(
            _drain_stderr(self._proc), name="acp-stderr-drain"
        )

        await self._conn.initialize(protocol_version=PROTOCOL_VERSION)
        session = await self._conn.new_session(cwd=str(self.cwd), mcp_servers=[])
        self._session_id = session.session_id
        logger.info("ACP session ready id=%s", self._session_id)
        return self

    async def __aexit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        del exc_type, exc, tb
        try:
            if self._cm is not None:
                await self._cm.__aexit__(None, None, None)
        finally:
            if self._stderr_task is not None:
                self._stderr_task.cancel()
                with contextlib.suppress(asyncio.CancelledError, Exception):
                    await self._stderr_task

    async def prompt(self, text: str) -> PromptOutcome:
        if self._conn is None or self._session_id is None:
            raise RuntimeError("ACP session is not started")

        self._client.messages.clear()
        self._client.thoughts.clear()

        response = await self._conn.prompt(
            session_id=self._session_id,
            prompt=[text_block(text)],
        )
        stop = getattr(response, "stop_reason", None) or "unknown"
        stop_s = stop.value if hasattr(stop, "value") else str(stop)
        answer = ""
        if self._client.messages:
            answer = self._client.messages[-1].strip()
        return PromptOutcome(
            stop_reason=stop_s,
            answer=answer,
            messages=list(self._client.messages),
            thoughts=list(self._client.thoughts),
        )


async def _drain_stderr(proc: Any) -> None:
    stderr = getattr(proc, "stderr", None)
    if stderr is None:
        return
    try:
        while True:
            line = await stderr.readline()
            if not line:
                break
            text = line.decode("utf-8", errors="replace").rstrip()
            if text:
                logger.debug("acp.stderr: %s", text)
    except asyncio.CancelledError:
        raise
    except Exception as err:  # noqa: BLE001 — best-effort drain
        logger.debug("stderr drain stopped: %s", err)
