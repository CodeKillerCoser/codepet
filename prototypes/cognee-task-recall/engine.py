"""Cognee integration, isolated to the probe's own directory and model cache."""

from __future__ import annotations

import dataclasses
import inspect
import json
import os
import re
import time
from collections import Counter
from pathlib import Path
from uuid import UUID
from urllib.parse import unquote, urlparse

DEFAULT_EMBEDDER = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
MLX_LLM = "ddalcu/Qwen3.8-27B-MLX-Serve-4bit"
MLX_EMBEDDER = "Qwen3-Embedding-0.6B-8bit"
UUID_PATTERN = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", re.I)


def configure(run_root: Path, cache_root: Path, embedder: str, *,
              backend: str = "gliner", endpoint: str = "", llm_model: str = MLX_LLM) -> None:
    settings = {
        "SYSTEM_ROOT_DIRECTORY": str(run_root / "system"),
        "DATA_ROOT_DIRECTORY": str(run_root / "data"),
        "CACHE_ROOT_DIRECTORY": str(run_root / "cache"),
        "HF_HOME": str(cache_root / "huggingface"),
        "FASTEMBED_CACHE_PATH": str(cache_root / "fastembed"),
        "GRAPH_DATABASE_PROVIDER": "ladybug",
        "VECTOR_DB_PROVIDER": "lancedb",
        "DB_PROVIDER": "sqlite",
        "GRAPH_EXTRACTOR": "gliner_demo",
        "GLINER_INFERENCE_THREADS": "1",
        "EMBEDDING_PROVIDER": "fastembed",
        "EMBEDDING_MODEL": embedder,
        "EMBEDDING_DIMENSIONS": "384" if "MiniLM" in embedder else "512",
        "EMBEDDING_MAX_COMPLETION_TOKENS": "512",
        "TOKENIZERS_PARALLELISM": "false",
        "OMP_NUM_THREADS": "4",
        "TELEMETRY_DISABLED": "1",
        "RUNTIME__DLTHUB_TELEMETRY": "false",
        "HF_HUB_DISABLE_TELEMETRY": "1",
        "HF_HUB_DISABLE_XET": "1",
        "IMPROVE_AUTO_ENABLED": "false",
        "ENABLE_BACKEND_ACCESS_CONTROL": "false",
        "LOG_LEVEL": "WARNING",
        # The default probe uses no cloud model, even if the caller has keys.
        "LLM_API_KEY": "",
        "OPENAI_API_KEY": "",
    }
    if backend == "mlx":
        from mlx_gateway import loopback_base
        endpoint = loopback_base(endpoint)
        settings.update({
            "GRAPH_EXTRACTOR": "llm", "LLM_PROVIDER": "custom",
            "LLM_MODEL": f"openai/{llm_model}", "LLM_ENDPOINT": endpoint,
            "LLM_API_KEY": "mlx-local", "LLM_TEMPERATURE": "0",
            "LLM_MAX_COMPLETION_TOKENS": "4096",
            "LLM_ARGS": json.dumps({"extra_body": {"chat_template_kwargs": {"enable_thinking": False}},
                                    "timeout": 600, "num_retries": 0}),
            "GRAPH_PROMPT_PATH": str(Path(__file__).with_name("development_graph_prompt.txt").resolve()),
            "STRUCTURED_OUTPUT_FRAMEWORK": "litellm_native",
            "EMBEDDING_PROVIDER": "openai_compatible", "EMBEDDING_ENDPOINT": endpoint,
            "EMBEDDING_API_KEY": "mlx-local", "EMBEDDING_DIMENSIONS": "1024",
            "EMBEDDING_MAX_COMPLETION_TOKENS": "4096", "EMBEDDING_ENCODING_FORMAT": "float",
            "EMBEDDING_BATCH_SIZE": "8", "EMBEDDING_CONCURRENCY": "1",
            "SUMMARY_METHOD": "from_extraction", "COGNEE_SKIP_CONNECTION_TEST": "true",
        })
        tokenizer = Path.home() / ".mlx-serve/models/mlx-community/Qwen3-Embedding-0.6B-8bit"
        if (tokenizer / "tokenizer.json").exists():
            settings["HUGGINGFACE_TOKENIZER"] = str(tokenizer)
    os.environ.update(settings)
    for path in (run_root, cache_root):
        path.mkdir(parents=True, exist_ok=True)


def plain(value):
    if dataclasses.is_dataclass(value):
        return plain(dataclasses.asdict(value))
    if hasattr(value, "model_dump"):
        return plain(value.model_dump(mode="json"))
    if isinstance(value, dict):
        return {str(key): plain(item) for key, item in value.items()
                if str(key).lower() not in {"embedding", "embeddings", "vector", "vectors",
                                            "text_embedding", "name_embedding", "fact_embedding"}}
    if isinstance(value, (list, tuple, set)):
        return [plain(item) for item in value]
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    return str(value)


def save_json(path: Path, value) -> None:
    path.write_text(json.dumps(plain(value), ensure_ascii=False, indent=2), encoding="utf-8")


def progress(phase: str, **fields) -> None:
    print(json.dumps({"phase": phase, **fields}, ensure_ascii=False), flush=True)


async def warm_models() -> dict:
    from fastembed import TextEmbedding
    from cognee.tasks.graph.gliner_demo.extractor import DEFAULT_MODEL, get_extractor

    started = time.perf_counter()
    model = os.environ["EMBEDDING_MODEL"]
    progress("download_embedding", model=model)
    embedder = TextEmbedding(model_name=model, cache_dir=os.environ["FASTEMBED_CACHE_PATH"], threads=4)
    list(embedder.embed(["任务执行记录 TaskRun"], batch_size=1))
    progress("download_extractor", model=DEFAULT_MODEL)
    extractor = await get_extractor()
    return {"embeddingModel": model, "extractorModel": DEFAULT_MODEL,
            "extractorClass": type(extractor).__name__, "seconds": time.perf_counter() - started}


async def ingest(documents: list[dict], dataset_name: str) -> dict:
    import cognee
    from cognee.tasks.ingestion.data_item import DataItem

    items = [DataItem(
        data=doc["text"], label=doc["taskTitle"], literal_text=True,
        external_metadata={key: doc[key] for key in (
            "runId", "taskId", "workspace", "sourceRevision", "contentHash", "updatedAt"
        )},
    ) for doc in documents]
    started = time.perf_counter()
    result = await cognee.remember(
        items, dataset_name=dataset_name, self_improvement=False, raise_on_error=True,
        chunks_per_batch=1,
    )
    if result.status != "completed":
        raise RuntimeError(f"Ingestion did not complete: {result.status}")
    return {"datasetId": str(result.dataset_id), "datasetName": dataset_name,
            "items": plain(result.items), "seconds": time.perf_counter() - started}


async def graph_snapshot(dataset_id: str) -> dict:
    from cognee.api.v1.visualize import visualize_graph_json
    from cognee.modules.users.methods import get_default_user

    payload = await visualize_graph_json(dataset=dataset_id, user=await get_default_user(), full=True)
    return plain(payload)


async def graph_html(dataset_id: str, path: Path) -> None:
    from cognee import visualize_graph

    await visualize_graph(str(path), dataset=dataset_id, full=True)


def candidate_ids(payload, documents: list[dict], data_ids: dict) -> list[str]:
    """Resolve only explicit IDs/provenance. Never infer ownership from title overlap."""
    allowed = {doc["runId"] for doc in documents}
    rows = plain(payload)
    if isinstance(rows, list) and rows and all(isinstance(row, dict) for row in rows):
        # Context strings concatenate chunks and can omit their metadata. Rank
        # from retrieved objects first; evidence is a provenance fallback.
        ordered = [row.get("objects_result") for row in rows]
        ordered += [row.get("evidence") for row in rows]
        text = json.dumps(ordered, ensure_ascii=False)
    else:
        text = json.dumps(rows, ensure_ascii=False)
    found = []
    for value in UUID_PATTERN.findall(text):
        resolved = value if value in allowed else data_ids.get(value, [])
        for run_id in ([resolved] if isinstance(resolved, str) else resolved):
            if run_id in allowed and run_id not in found:
                found.append(run_id)
    return found


def ownership_map(graph: dict, data_ids: dict[str, str]) -> dict:
    """Only structural provenance edges may spread ownership, never entity relations."""
    owners = {key: [value] for key, value in data_ids.items()}
    node_types = {str(node["id"]): node.get("type") for node in graph.get("nodes", [])}
    for node in graph.get("nodes", []):
        metadata = node.get("external_metadata")
        if isinstance(metadata, str):
            metadata = json.loads(metadata)
        if isinstance(metadata, dict) and metadata.get("runId"):
            owners[str(node["id"])] = [metadata["runId"]]
    for _ in range(3):
        for edge in graph.get("links", []):
            source, target = str(edge["source"]), str(edge["target"])
            if node_types.get(source) != "DocumentChunk":
                continue
            if node_types.get(target) not in {"Entity", "TextSummary"}:
                continue
            for run_id in owners.get(source, []):
                if run_id not in owners.setdefault(target, []):
                    owners[target].append(run_id)
    return owners


def local_file(location: str) -> Path:
    parsed = urlparse(location)
    if parsed.scheme and parsed.scheme != "file":
        raise ValueError(f"Probe expects a local stored document, got {parsed.scheme}")
    return Path(unquote(parsed.path)) if parsed.scheme else Path(location)


async def search_routes(query: str, dataset_id: str, documents: list[dict], data_ids: dict) -> dict:
    import cognee
    from cognee import SearchType

    routes = {}
    for name, query_type in (
        ("semantic", SearchType.CHUNKS),
        ("lexical", SearchType.CHUNKS_LEXICAL),
        ("graph", SearchType.GRAPH_COMPLETION),
    ):
        started = time.perf_counter()
        try:
            if name == "lexical":
                from cognee.modules.retrieval.bm25_retriever import BM25ChunksRetriever
                # Cognee 1.6.10's public search factory forwards only top_k
                # for this type. The pinned native retriever exposes scores.
                hits = await BM25ChunksRetriever(top_k=8, with_scores=True).get_retrieved_objects(query)
                result = [{"objects_result": hits, "evidence": []}]
            else:
                result = await cognee.search(
                    query_text=query, query_type=query_type, dataset_ids=[UUID(dataset_id)],
                    top_k=8, only_context=True, include_references=True, verbose=True,
                )
            raw = plain(result)
            mapped = candidate_ids(raw, documents, data_ids)
            extra = {}
            if name == "lexical":
                # Native BM25 returns top-k even when every score is zero.
                # Preserve its raw output, but a zero score is not a keyword hit.
                positive_rows, zero_scores = [], 0
                for row in raw:
                    positive = []
                    for hit in row.get("objects_result") or []:
                        if isinstance(hit, list) and len(hit) == 2 and isinstance(hit[1], (int, float)):
                            if hit[1] > 0:
                                positive.append(hit)
                            else:
                                zero_scores += 1
                    positive_rows.append({"objects_result": positive, "evidence": []})
                extra = {"nativeRunIds": mapped, "discardedZeroScoreChunks": zero_scores}
                mapped = candidate_ids(positive_rows, documents, data_ids)
            routes[name] = {"runIds": mapped, "raw": raw, **extra,
                            "seconds": time.perf_counter() - started}
        except Exception as error:
            routes[name] = {"runIds": [], "error": f"{type(error).__name__}: {error}",
                            "seconds": time.perf_counter() - started}
    return routes


async def document_rows(dataset_id: str) -> list[dict]:
    from sqlalchemy import select
    from cognee.infrastructure.databases.relational import get_relational_engine
    from cognee.modules.data.models import Data

    async with get_relational_engine().get_async_session() as session:
        rows = (await session.execute(select(Data).where(Data.dataset_id == UUID(dataset_id)))).scalars().all()
        return [{"dataId": str(row.id), "name": row.name,
                 "externalMetadata": plain(row.external_metadata),
                 "rawDataLocation": str(row.raw_data_location), "contentHash": row.content_hash}
                for row in rows]


async def update_document(data_id: str, dataset_id: str, text: str) -> dict:
    from cognee import update

    started = time.perf_counter()
    result = await update(data_id=UUID(data_id), data=text, dataset_id=UUID(dataset_id))
    return {"result": plain(result), "seconds": time.perf_counter() - started}


async def stored_chunk_ids(ids: list[str]) -> set[str]:
    from cognee.infrastructure.databases.vector import get_vector_engine

    vector = get_vector_engine()
    if inspect.isawaitable(vector):
        vector = await vector
    rows = await vector.retrieve("DocumentChunk_text", ids)
    return {str(row.id) for row in rows}


def fuse(routes: dict, documents: list[dict], workspace: str) -> list[dict]:
    """Fixed weights, chosen before the benchmark. Workspace is a soft prior."""
    lanes = {name: value["runIds"] for name, value in routes.items()}
    lanes["recency"] = [doc["runId"] for doc in sorted(documents, key=lambda doc: doc["updatedAt"], reverse=True)]
    lanes["workspace"] = [doc["runId"] for doc in documents if doc["workspace"] == workspace]
    weights = {"semantic": 1.0, "lexical": 0.7, "graph": 0.4, "recency": 0.04, "workspace": 0.05}
    scores, evidence = Counter(), {}
    # Metadata alone must not manufacture query relevance.
    eligible = {run_id for name in routes for run_id in lanes[name]}
    for name, run_ids in lanes.items():
        for rank, run_id in enumerate(run_ids, 1):
            if run_id in eligible:
                scores[run_id] += weights[name] / (10 + rank)
                evidence.setdefault(run_id, []).append({"route": name, "rank": rank})
    return [{"runId": run_id, "score": score, "evidence": evidence[run_id]}
            for run_id, score in scores.most_common()]
