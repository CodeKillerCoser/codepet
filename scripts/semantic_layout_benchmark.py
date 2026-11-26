"""Compare pinned native and accelerated computation on synthetic vectors only.

Run with the sidecar's Python and --bundle <unpacked bundle>; writes no index.
"""
import argparse
import importlib
import json
import os
from pathlib import Path
import statistics
import sys
import time

parser = argparse.ArgumentParser()
parser.add_argument('--bundle', type=Path, required=True)
parser.add_argument('--counts', default='700,2000')
parser.add_argument('--repeats', type=int, default=3)
parser.add_argument('--output', type=Path, default=Path('artifacts/graph-performance/computation-benchmark.json'))
args = parser.parse_args()
for name in ['OMP_NUM_THREADS', 'OPENBLAS_NUM_THREADS', 'VECLIB_MAXIMUM_THREADS', 'MKL_NUM_THREADS']:
    os.environ.setdefault(name, '4')
os.environ['AWS_EC2_METADATA_DISABLED'] = 'true'
sys.path[:0] = [str(Path(__file__).resolve().parents[1] / 'sidecars/task-recall'), str(args.bundle.resolve() / 'packages')]
import numpy as np
from codepet_recall.semantic_layout import deoverlap

layout = importlib.import_module('cognee.modules.visualization.layouts.semantic_layout')
clusters = importlib.import_module('cognee.modules.visualization.semantic_clusters')
original = layout._deoverlap
results = []
try:
    for count in map(int, args.counts.split(',')):
        if not 2 <= count <= 2000 or args.repeats < 1:
            raise ValueError('Use 2–2000 nodes and at least one repeat')
        rng = np.random.default_rng(42)
        nodes = [{'id':str(i), 'type':'Entity', 'name':f'Synthetic {i}'} for i in range(count)]
        edge_count = 1600 if count == 700 else count * 3
        links = [{'source':str(i % count), 'target':str((i * 29 + 1) % count)} for i in range(edge_count)]
        vectors = {str(i):row.tolist() for i, row in enumerate(rng.standard_normal((count, 1024)))}
        samples = {'native':[], 'accelerated':[]}
        baseline = None
        maximum_delta = 0.0
        for repeat in range(args.repeats):
            order = [('native', original), ('accelerated', deoverlap)]
            if repeat % 2:
                order.reverse()
            for name, function in order:
                layout._deoverlap = function
                start = time.perf_counter()
                positions = layout.compute_positions(nodes, links, vectors)
                groups = clusters.compute_clusters(nodes, vectors)
                samples[name].append(time.perf_counter() - start)
                if baseline is None:
                    baseline = (positions, groups)
                maximum_delta = max(maximum_delta, max(abs(p[axis] - positions[node][axis])
                                    for node, p in baseline[0].items() for axis in ['x', 'y']))
                assert maximum_delta < 1e-10 and groups == baseline[1], 'Native semantic output changed'
        medians = {name:statistics.median(values) for name, values in samples.items()}
        row = {'synthetic':True, 'nodes':count, 'edges':edge_count, 'dimensions':1024,
               'seconds':samples, 'medianSeconds':medians,
               'speedup':medians['native'] / medians['accelerated'], 'maxCoordinateDelta':maximum_delta}
        results.append(row)
        print(json.dumps(row), flush=True)
finally:
    layout._deoverlap = original
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(results, indent=2))
