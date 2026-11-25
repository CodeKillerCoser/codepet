import json
import sys
import types
import unittest
import uuid
from pathlib import Path
from unittest.mock import AsyncMock, patch
from codepet_recall.engine import Engine


class ManualEngineTests(unittest.IsolatedAsyncioTestCase):
    async def test_rebuild_uses_native_single_document_update_without_disabling_dataset_incrementality(self):
        update = AsyncMock(return_value={"status":"full_rebuild"})
        modules = {"cognee":types.SimpleNamespace(update=update),
                   "cognee.tasks.ingestion.data_item":types.SimpleNamespace(DataItem=lambda **kwargs:kwargs)}
        data_id, dataset_id = str(uuid.uuid4()), str(uuid.uuid4())
        doc = {"text":"task run", "taskTitle":"topic", "runId":str(uuid.uuid4()),
               "taskId":str(uuid.uuid4()), "contentHash":"version"}
        row = {"document":json.dumps(doc), "data_id":data_id, "deleted":False, "rebuild":True}
        engine = Engine(Path("unused"), {})
        engine.initialized = True
        engine.dataset = dataset_id
        with patch.dict(sys.modules, modules):
            self.assertEqual(await engine.process(row), data_id)
            options = update.call_args.kwargs
            self.assertFalse(options["chunk_level_diff"])
            self.assertFalse(options["data_cache"])
            self.assertNotIn("incremental_loading", options)
            self.assertEqual(str(options["data_id"]), data_id)
            row["rebuild"] = False
            await engine.process(row)
            self.assertNotIn("chunk_level_diff", update.call_args.kwargs)
