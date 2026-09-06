#!/usr/bin/env python3
"""Summarize aggregate-only Frame V1 benchmark data; never logs payload text."""
import collections
import json
import statistics
import sys
from pathlib import Path


def summarize(data):
    groups = collections.defaultdict(list)
    for row in data['samples']:
        groups[(row['direction'], row['kind'], row['payloadBytes'], row['rateBytesPerSecond'])].append(row)
    result = []
    for (direction, kind, size, rate), rows in groups.items():
        entry = dict(direction=direction, kind=kind, payloadBytes=size, rateBytesPerSecond=rate, n=len(rows))
        for metric in ('smallRpcMs', 'pingMs', 'eventMs', 'triggerMs'):
            values = [row[metric] for row in rows]
            entry[metric] = dict(median=statistics.median(values), min=min(values), max=max(values))
        io = [event['elapsedMs'] for row in rows for event in row['trace']
              if event.get('bench') == 'io_end' and event.get('primary')]
        entry['primaryIoMs'] = dict(median=statistics.median(io), min=min(io), max=max(io))
        entry['frameBytes'] = next(event['bytes'] for event in rows[0]['trace']
                                   if event.get('bench') == 'io_end' and event.get('primary'))
        entry['largeCompletionMs'] = statistics.median(row['large']['completionMs'] for row in rows)
        result.append(entry)
    return result


def main():
    source = Path(sys.argv[1])
    data = json.loads(source.read_text())
    summary = summarize(data)
    target = source.with_name(source.stem + '-summary.json')
    target.write_text(json.dumps(summary, indent=2) + '\n')
    print('direction kind MiB rateMiB/s wireMiB smallMedianMs smallMaxMs pingMedianMs ioMedianMs')
    for row in summary:
        print(row['direction'], row['kind'], round(row['payloadBytes']/2**20, 3),
              row['rateBytesPerSecond']/2**20, round(row['frameBytes']/2**20, 4),
              round(row['smallRpcMs']['median'], 3), round(row['smallRpcMs']['max'], 3),
              round(row['pingMs']['median'], 3), round(row['primaryIoMs']['median'], 3))


if __name__ == '__main__':
    main()
