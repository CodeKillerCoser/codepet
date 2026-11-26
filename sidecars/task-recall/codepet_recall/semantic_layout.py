"""Exact-radius acceleration for Cognee's seeded 2-D separation pass.

Keep its PCA, clustering, seed, 40 rounds and displacement formula. A spatial
hash only removes pairs that cannot contribute. Dense cells use the original
vectorized operation rather than falling back to quadratic Python loops.
"""
import inspect
import numpy as np


def _dense_step(points, minimum):
    delta = points[:, None, :] - points[None, :, :]
    distance = np.sqrt((delta ** 2).sum(axis=2))
    np.fill_diagonal(distance, minimum)
    close = distance < minimum
    if not close.any():
        return None
    safe = np.where(distance == 0, 1.0, distance)
    push = np.where(close, (minimum - distance) / safe, 0.0)
    return points + (delta * push[:, :, None]).sum(axis=1) * 0.5


def deoverlap(ordered_ids, positioned, min_dist, rng, iterations=40):
    if len(ordered_ids) < 2:
        return positioned
    points = np.array([positioned[node] for node in ordered_ids], dtype=float)
    points = points + rng.uniform(-min_dist / 4, min_dist / 4, size=points.shape)
    finite = np.isfinite(points).all()
    for _ in range(iterations):
        if min_dist <= 0:
            break
        if not finite:
            updated = _dense_step(points, min_dist)
            if updated is None:
                break
            points = updated
            continue
        coordinates = np.floor(points / min_dist).astype(np.int64)
        cells = {}
        for i, coordinate in enumerate(coordinates):
            cells.setdefault(tuple(coordinate), []).append(i)
        # Estimate candidate work before constructing pairs. Collapsed points
        # are cheaper in NumPy than in a large Python neighbor list.
        candidate_count = sum(len(members) * len(cells.get((x + dx, y + dy), ()))
                              for (x, y), members in cells.items()
                              for dx in (-1, 0, 1) for dy in (-1, 0, 1))
        if max(map(len, cells.values())) > 64 or candidate_count > len(points) ** 2 / 4:
            updated = _dense_step(points, min_dist)
            if updated is None:
                break
            points = updated
            continue
        left, right = [], []
        for i, (x, y) in enumerate(coordinates):
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    for j in cells.get((x + dx, y + dy), ()):
                        if j > i:
                            left.append(i)
                            right.append(j)
        if not left:
            break
        pairs = np.column_stack((left, right))
        pairs = pairs[np.lexsort((pairs[:, 1], pairs[:, 0]))]
        delta = points[pairs[:, 0]] - points[pairs[:, 1]]
        distance = np.sqrt((delta ** 2).sum(axis=1))
        close = distance < min_dist
        if not close.any():
            break
        pairs, delta, distance = pairs[close], delta[close], distance[close]
        safe = np.where(distance == 0, 1.0, distance)
        push = delta * ((min_dist - distance) / safe)[:, None]
        shift = np.zeros_like(points)
        np.add.at(shift, pairs[:, 0], push)
        np.add.at(shift, pairs[:, 1], -push)
        points = points + shift * 0.5
    return {node: points[i] for i, node in enumerate(ordered_ids)}


def install():
    from cognee.modules.visualization.layouts import semantic_layout
    current = semantic_layout._deoverlap
    if current is deoverlap:
        return
    expected = ['ordered_ids', 'positioned', 'min_dist', 'rng', 'iterations']
    if list(inspect.signature(current).parameters) != expected:
        raise RuntimeError('Cognee semantic layout changed; review the pinned accelerator')
    semantic_layout._deoverlap = deoverlap
