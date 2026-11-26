import json
from pathlib import Path
import tempfile
import unittest
from codepet_recall.graph_cache import GraphCache
from codepet_recall.published_preview import PublishedPreview


class PublishedPreviewTests(unittest.TestCase):
    def test_writer_invalidation_cannot_remove_published_preview_and_restart_keeps_it(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'dataset.json').write_text(json.dumps({'id':'dataset'}))
            old = {'nodes':[{'id':'completed'}],'links':[],'previewVersion':'one'}
            published = PublishedPreview(root)
            published.publish('dataset', old, {**old,'semantic_positions':{'completed':{'x':1,'y':2}}})
            working = GraphCache(root)
            working.put('dataset', False, old)
            working.invalidate()
            self.assertEqual(published.get(), old)
            self.assertEqual(PublishedPreview(root).get(), old)
            new = {**old,'nodes':[{'id':'next-completed'}],'previewVersion':'two'}
            published.publish('dataset', new)
            self.assertEqual(published.get(True), new, 'old semantic positions must not mix with the next graph')
            self.assertEqual(PublishedPreview(root).get(True), new)
            (root / 'dataset.json').write_text(json.dumps({'id':'other-dataset'}))
            self.assertIsNone(PublishedPreview(root).get())
