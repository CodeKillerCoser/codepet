import importlib
import unittest
from unittest.mock import patch
import numpy as np
from codepet_recall.semantic_layout import deoverlap, install

native = importlib.import_module('cognee.modules.visualization.layouts.semantic_layout')
reference = native._deoverlap


class SemanticLayoutTests(unittest.TestCase):
    def compare(self, points, minimum=.02, rounds=40):
        ids = [str(i) for i in range(len(points))]
        positions = dict(zip(ids, points))
        before = np.array(points, copy=True)
        original_rng, fast_rng = np.random.default_rng(42), np.random.default_rng(42)
        expected = reference(ids, positions, minimum, original_rng, iterations=rounds)
        actual = deoverlap(ids, positions, minimum, fast_rng, iterations=rounds)
        np.testing.assert_allclose(list(actual.values()), list(expected.values()), rtol=0, atol=1e-11)
        np.testing.assert_array_equal(points, before)
        np.testing.assert_array_equal(original_rng.random(10), fast_rng.random(10))
        repeated = deoverlap(ids, positions, minimum, np.random.default_rng(42), iterations=rounds)
        np.testing.assert_array_equal(list(actual.values()), list(repeated.values()))

    def test_actual_scale_700_and_bounded_2000_match_native_seeded_layout(self):
        rng = np.random.default_rng(12)
        for count in [700, 2000]:
            with self.subTest(count=count):
                self.compare(rng.normal(0, .3, (count, 2)))

    def test_dense_overlap_and_grid_boundaries_preserve_results(self):
        self.compare(np.zeros((250, 2)))
        self.compare(np.array([[-.02, 0], [-.00001, 0], [0, 0], [.01999, 0], [.02, .02], [.04, .04]]))
        self.compare(np.array([[0., 0.], [1., 1.]]), minimum=0)
        self.compare(np.array([[0., 0.], [.001, .001]]), rounds=0)

    def test_empty_and_single_node_preserve_identity(self):
        for positions in [{}, {'one':np.array([0., 0.])}]:
            self.assertIs(deoverlap(list(positions), positions, .02, np.random.default_rng(42)), positions)

    def test_installs_only_the_separation_pass_and_rejects_framework_contract_change(self):
        positions, clusters = native.compute_positions, native._pca_2d
        with patch.object(native, '_deoverlap', reference):
            install(); install()
            self.assertIs(native._deoverlap, deoverlap)
            self.assertIs(native.compute_positions, positions)
            self.assertIs(native._pca_2d, clusters)
        with patch.object(native, '_deoverlap', lambda changed_contract:None):
            with self.assertRaisesRegex(RuntimeError, 'changed'):
                install()
