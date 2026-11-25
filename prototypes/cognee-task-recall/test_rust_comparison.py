"""Regression checks for evaluation mistakes found during the Rust comparison."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("codepet_rust_probe", Path(__file__).parent / "rust/run.py")
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class RustComparisonTests(unittest.TestCase):
    def test_framework_hybrid_does_not_enter_fixed_fusion(self):
        documents = [
            {"runId":"A", "workspace":"x", "updatedAt":1},
            {"runId":"B", "workspace":"x", "updatedAt":2},
        ]
        routes = {"semantic":{"runIds":["A"]}, "lexical":{"runIds":[]},
                  "graph":{"runIds":[]}, "nativeHybrid":{"runIds":["B"]}}
        self.assertEqual([r["runId"] for r in probe.comparison_fuse(routes, documents, "x")], ["A"])

    def test_control_entity_names_follow_framework_case_normalization(self):
        graph = {"nodes":[{"id":"one", "name":"ProbeOldLibrary", "type":"Entity"},
                           {"id":"two", "name":"probesharedlibrary", "type":"Entity"},
                           {"id":"doc", "name":"ProbeNewLibrary", "type":"TextDocument"}]}
        self.assertEqual(probe.node_names(graph), {"probeoldlibrary":"one", "probesharedlibrary":"two"})


if __name__ == "__main__":
    unittest.main()
