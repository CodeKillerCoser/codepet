"""Attach attempt identity to SDK calls only at our private Gateway boundary.

Cognee uses both LiteLLM and OpenAI SDK clients. Their httpx/httpx2 send boundaries
carries an async ContextVar into child tasks without changing extraction behavior
or mutating cached model clients. Other hosts and unscoped search calls are untouched.
"""
from contextvars import ContextVar
from urllib.parse import urlsplit

attempt_id = ContextVar("codepet_processing_attempt", default=None)
_installed = False


def install(endpoint, token):
    global _installed
    if _installed:
        return
    import importlib
    expected = urlsplit(endpoint)
    def wrap(original):
        async def send(client, request, *args, **kwargs):
            job = attempt_id.get()
            actual = urlsplit(str(request.url))
            if (job and (actual.scheme, actual.netloc) == (expected.scheme, expected.netloc)
                    and actual.path.startswith(expected.path.rstrip("/") + "/")
                    and request.headers.get("authorization") == "Bearer " + token):
                request.headers["X-CodePet-Processing-Job"] = job
            return await original(client, request, *args, **kwargs)
        return send

    # Pinned OpenAI SDK uses httpx2; LiteLLM still uses httpx. Cover both without
    # changing provider payloads, cached clients or Cognee's processing pipeline.
    for name in ("httpx", "httpx2"):
        module = importlib.import_module(name)
        module.AsyncClient.send = wrap(module.AsyncClient.send)
    _installed = True
