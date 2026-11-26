"""Pinned Cognee owns document processing, embeddings, graph and replacement."""
import asyncio
import hashlib
import json
import os
from pathlib import Path
from uuid import UUID, uuid4
from .graph_cache import GraphCache


def plain(value):
    if hasattr(value, "model_dump"):
        return plain(value.model_dump(mode="json"))
    if isinstance(value, dict):
        return {str(k):plain(v) for k,v in value.items() if "embedding" not in str(k).lower()}
    if isinstance(value, (list, tuple)):
        return [plain(v) for v in value]
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    return str(value)


class Engine:
    def __init__(self, root, config):
        self.root = root
        self.config = config
        self.dataset = None
        self.owners = {}
        self.initialized = False
        self.preview_lock = asyncio.Lock()
        self.preview = GraphCache(root / config["fingerprint"]) if config.get("fingerprint") else None

    async def initialize(self):
        if self.initialized:
            return
        endpoint, key = self.config["gatewayEndpoint"], self.config["gatewayToken"]
        # Profile changes get an isolated index; embedding dimensions cannot mix.
        fingerprint = self.config["fingerprint"]
        root = self.root / fingerprint
        settings = {
            "SYSTEM_ROOT_DIRECTORY":str(root / "system"), "DATA_ROOT_DIRECTORY":str(root / "data"),
            "CACHE_ROOT_DIRECTORY":str(root / "cache"), "GRAPH_DATABASE_PROVIDER":"ladybug",
            "VECTOR_DB_PROVIDER":"lancedb", "DB_PROVIDER":"sqlite", "GRAPH_EXTRACTOR":"llm",
            # Rust already owns one isolated writer; avoid nested native DB
            # workers retaining file locks after the sidecar owner exits.
            "VECTOR_DB_SUBPROCESS_ENABLED":"false", "GRAPH_DATABASE_SUBPROCESS_ENABLED":"false",
            "LLM_PROVIDER":"custom", "LLM_MODEL":"openai/" + self.config["llm"]["model"],
            "LLM_ENDPOINT":endpoint, "LLM_API_KEY":key, "LLM_TEMPERATURE":"0",
            "LLM_MAX_COMPLETION_TOKENS":"4096", "LLM_ARGS":json.dumps({"timeout":600,"num_retries":0}),
            "STRUCTURED_OUTPUT_FRAMEWORK":"litellm_native", "EMBEDDING_PROVIDER":"openai_compatible",
            "EMBEDDING_MODEL":self.config["embedding"]["model"], "EMBEDDING_ENDPOINT":endpoint,
            "EMBEDDING_API_KEY":key, "EMBEDDING_DIMENSIONS":str(self.config["embedding"]["dimensions"]),
            "EMBEDDING_MAX_COMPLETION_TOKENS":"4096", "EMBEDDING_BATCH_SIZE":"8",
            "EMBEDDING_CONCURRENCY":"1", "EMBEDDING_ENCODING_FORMAT":"float",
            "SUMMARY_METHOD":"from_extraction", "COGNEE_SKIP_CONNECTION_TEST":"true",
            "TELEMETRY_DISABLED":"1", "HF_HUB_DISABLE_TELEMETRY":"1", "HF_HUB_OFFLINE":"1",
            "LITELLM_LOCAL_MODEL_COST_MAP":"true", "IMPROVE_AUTO_ENABLED":"false",
            "ENABLE_BACKEND_ACCESS_CONTROL":"false", "LOG_LEVEL":"ERROR",
            "GRAPH_PROMPT_PATH":str(Path(__file__).with_name("graph_prompt.txt")),
            "TIKTOKEN_CACHE_DIR":str(Path(__file__).resolve().parents[1] / "tokenizer-cache"),
        }
        os.environ.update(settings)
        self.initialized = True
        info = root / "dataset.json"
        self.info = info
        if info.exists():
            self.dataset = json.loads(info.read_text())["id"]

    async def process(self, row):
        async with self.preview_lock:
            if self.preview:
                self.preview.invalidate()
            try:
                return await self._process(row)
            finally:
                if self.preview:
                    self.preview.invalidate()

    async def _process(self, row):
        await self.initialize()
        import cognee
        from cognee.tasks.ingestion.data_item import DataItem
        doc = json.loads(row["document"])
        if row["deleted"]:
            if row["data_id"] and self.dataset:
                await cognee.delete(UUID(row["data_id"]), UUID(self.dataset), mode="hard")
            return None
        item = DataItem(data=doc["text"], label=doc.get("taskTitle", "Task run"), literal_text=True,
                        external_metadata={k:doc[k] for k in ("runId", "taskId", "contentHash")})
        if row["data_id"] and self.dataset:
            # Cognee resets this document's pipeline stamp. Keep incremental
            # loading enabled so unrelated finished documents are not rebuilt.
            options = {"chunk_level_diff":False, "data_cache":False} if row.get("rebuild") else {}
            result = await cognee.update(data_id=UUID(row["data_id"]), data=item, dataset_id=UUID(self.dataset), **options)
            if result.get("status") == "failed":
                raise RuntimeError("Cognee update failed")
            return row["data_id"]
        result = await cognee.remember([item], dataset_name="codepet_task_runs", self_improvement=False,
                                       raise_on_error=True, chunks_per_batch=1)
        if result.status != "completed":
            raise RuntimeError("Cognee ingestion incomplete")
        self.dataset = str(result.dataset_id)
        self.info.parent.mkdir(parents=True, exist_ok=True)
        self.info.write_text(json.dumps({"id":self.dataset}))
        # Resolve provenance from Cognee's persisted ingestion records, never title guessing.
        from sqlalchemy import select
        from cognee.infrastructure.databases.relational import get_relational_engine
        from cognee.modules.data.models import Data
        async with get_relational_engine().get_async_session() as session:
            rows = (await session.execute(select(Data).where(Data.dataset_id == UUID(self.dataset)))).scalars().all()
            for data in rows:
                metadata = data.external_metadata
                if isinstance(metadata, str):
                    metadata = json.loads(metadata)
                if metadata and metadata.get("runId") == doc["runId"]:
                    return str(data.id)
        raise RuntimeError("Cognee source provenance missing")

    async def result(self, document):
        from .results import processing_result
        return processing_result(await self.graph(), document)

    async def graph(self, semantic=False):
        await self.initialize()
        if not self.dataset:
            return {"nodes":[], "links":[]}
        async with self.preview_lock:
            base = self.preview.get(self.dataset, False) if self.preview else None
            cached = self.preview.get(self.dataset, semantic) if self.preview else None
            if cached is not None and (not semantic or (base is not None
                    and base["previewVersion"] == cached["previewVersion"])):
                return cached
            if semantic:
                from .semantic_layout import install
                install()
            from cognee.api.v1.visualize import visualize_graph_json, visualize_semantic_json
            from cognee.modules.users.methods import get_default_user
            user = await get_default_user()
            if base is None:
                base = plain(await visualize_graph_json(dataset=self.dataset, user=user, full=True))
                base["previewVersion"] = str(uuid4())
                if self.preview:
                    self.preview.put(self.dataset, False, base)
            if not semantic:
                return base
            result = {**base, **plain(await visualize_semantic_json(dataset=self.dataset, user=user, full=True))}
            if self.preview:
                self.preview.put(self.dataset, True, result)
            return result

    async def search(self, query, ready):
        await self.initialize()
        if not self.dataset or not ready:
            return {}, {}
        graph = await self.graph()
        owners, types = {}, {}
        for node in graph.get("nodes", []):
            key = str(node["id"])
            types[key] = node.get("type")
            meta = node.get("external_metadata") or {}
            if isinstance(meta, str):
                meta = json.loads(meta)
            run = ready.get(meta.get("runId"))
            if run and meta.get("contentHash") == run["contentHash"]:
                owners[key] = {run["runId"]}
        for edge in graph.get("links", []):
            source, target = str(edge["source"]), str(edge["target"])
            if types.get(source) == "DocumentChunk" and types.get(target) in ("Entity", "TextSummary"):
                owners.setdefault(target, set()).update(owners.get(source, set()))

        def resolve(value, found):
            if isinstance(value, dict):
                for key,item in value.items():
                    if key in ("node_id", "artifact_id", "id", "document_id") and isinstance(item, str):
                        for run_id in sorted(owners.get(item, set())):
                            if run_id not in found:
                                found.append(run_id)
                    elif key not in ("text", "user_prompt_result", "system_prompt_result", "context_result", "text_result"):
                        resolve(item, found)
            elif isinstance(value, (list, tuple)):
                for item in value:
                    resolve(item, found)

        import cognee
        from cognee import SearchType
        routes, errors = {}, {}
        for name, kind in (("semantic", SearchType.CHUNKS), ("graph", SearchType.GRAPH_COMPLETION)):
            try:
                result = await asyncio.wait_for(cognee.search(query_text=query, query_type=kind,
                    dataset_ids=[UUID(self.dataset)], top_k=12, only_context=True,
                    include_references=True, verbose=True), timeout=25)
                found = []
                resolve(plain(result), found)
                routes[name] = [{"runId":rid,"contentHash":ready[rid]["contentHash"]} for rid in found]
            except Exception as error:
                errors[name] = type(error).__name__
        return routes, errors
