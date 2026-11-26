"""Preview reads must not repeat native export/projection or survive index writes."""
import asyncio
import json
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import AsyncMock, patch
from codepet_recall.engine import Engine
from codepet_recall.graph_cache import GraphCache


class GraphPreviewTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.graph = AsyncMock(return_value={"nodes":[{"id":"native"}], "links":[]})
        self.semantic = AsyncMock(return_value={"semantic_positions":{"native":{"x":1,"y":2}}, "semantic_clusters":{"clusters":[]}})
        self.modules = patch.dict(sys.modules, {
            "cognee.api.v1.visualize":types.SimpleNamespace(visualize_graph_json=self.graph, visualize_semantic_json=self.semantic),
            "cognee.modules.users.methods":types.SimpleNamespace(get_default_user=AsyncMock(return_value="user")),
        })
        self.modules.start()
        self.accelerator = patch("codepet_recall.semantic_layout.install")
        self.accelerator.start()

    def tearDown(self):
        self.accelerator.stop()
        self.modules.stop()
        self.temp.cleanup()

    def engine(self, fingerprint="profile", dataset="dataset"):
        engine = Engine(self.root, {"fingerprint":fingerprint})
        engine.initialized = True
        engine.dataset = dataset
        return engine

    async def test_basic_read_never_projects_semantic_and_concurrent_reads_share_export(self):
        engine = self.engine()
        graphs = await asyncio.gather(engine.graph(), engine.graph(), engine.graph())
        self.graph.assert_awaited_once()
        self.semantic.assert_not_awaited()
        self.assertEqual(graphs[0], graphs[2])
        self.assertNotIn("semantic_positions", graphs[0])
        projected = await engine.graph(True)
        self.assertEqual(projected["previewVersion"], graphs[0]["previewVersion"])
        self.assertEqual(projected["semantic_positions"]["native"]["x"], 1)
        self.semantic.assert_awaited_once()
        self.assertEqual(await engine.graph(True), projected)
        self.graph.assert_awaited_once()

    async def test_restart_reads_private_cache_without_importing_native_modules(self):
        result = await self.engine().graph(True)
        self.graph.reset_mock(); self.semantic.reset_mock()
        with patch.dict(sys.modules, {"cognee.api.v1.visualize":None}):
            self.assertEqual(await self.engine().graph(True), result)
        self.graph.assert_not_awaited(); self.semantic.assert_not_awaited()
        if sys.platform != 'win32':
            self.assertEqual((self.root/'profile/preview-semantic.json').stat().st_mode & 0o777, 0o600)
        await self.engine(dataset="different").graph()
        await self.engine(fingerprint="different").graph()
        self.assertEqual(self.graph.await_count, 2)

    async def test_index_changes_and_partial_failure_invalidate_both_caches(self):
        engine = self.engine()
        old = await engine.graph(True)
        async def change(row):
            self.assertIsNone(engine.preview.get(engine.dataset, True))
            # Even an export produced before a partial write finishes must expire.
            engine.preview.put(engine.dataset, False, old)
            raise RuntimeError("partial native change")
        engine._process = change
        with self.assertRaises(RuntimeError):
            await engine.process({})
        self.assertIsNone(engine.preview.get(engine.dataset, False))
        new = await self.engine().graph(True)
        self.assertNotEqual(old["previewVersion"], new["previewVersion"])
        self.assertEqual(self.graph.await_count, 2)
        self.assertEqual(self.semantic.await_count, 2)

    async def test_projection_failure_keeps_basic_preview_and_can_retry(self):
        engine = self.engine()
        self.semantic.side_effect = [RuntimeError("projection unavailable"), self.semantic.return_value]
        with self.assertRaises(RuntimeError):
            await engine.graph(True)
        await engine.graph()
        await engine.graph(True)
        self.graph.assert_awaited_once()
        self.assertEqual(self.semantic.await_count, 2)

    async def test_corrupt_cache_falls_back_to_native_data(self):
        path = self.root/'profile/preview-graph.json'
        path.parent.mkdir()
        path.write_text('{broken')
        self.assertEqual((await self.engine().graph())["nodes"], [{"id":"native"}])
        saved = json.loads(path.read_text()); saved['version'] = 'obsolete'
        path.write_text(json.dumps(saved))
        await self.engine().graph()
        self.assertEqual(self.graph.await_count, 2)
