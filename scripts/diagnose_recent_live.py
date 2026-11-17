#!/usr/bin/env python3
"""Sample the running macOS Host, its Codex Provider, and native descendants.

Does not restart processes, issue prompts/RPCs, read conversation data or credentials.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import datetime
import json
from pathlib import Path
import platform
import subprocess
import tempfile
import time
import zipfile


def parse_processes(process_text):
    processes = {}
    for line in process_text.splitlines():
        parts = line.strip().split(None, 2)
        if len(parts) != 3:
            continue
        try:
            pid, parent = int(parts[0]), int(parts[1])
        except ValueError:
            continue
        processes[pid] = (parent, parts[2])
    return processes


def host_candidates(process_text):
    return {pid: command for pid, (_, command) in parse_processes(process_text).items()
            if Path(command).name.lower().replace('-', '').replace(' ', '')
            in {'codepet', 'codepethost', 'hangingmetal'}}


def discover_targets(process_text, host_pid=None):
    processes = parse_processes(process_text)
    hosts = ({host_pid} & processes.keys()) if host_pid is not None else set(host_candidates(process_text))
    targets = {pid: 'Host' for pid in hosts}

    def belongs_to_host(pid):
        visited = set()
        while pid in processes and pid not in visited:
            if pid in hosts:
                return True
            visited.add(pid)
            pid = processes[pid][0]
        return False

    providers = {pid for pid, (_, command) in processes.items()
                 if Path(command).name == 'codepet-provider-codex' and belongs_to_host(pid)}
    targets.update({pid: 'Provider' for pid in providers})
    descendants = set(providers)
    while True:
        children = {pid for pid, (parent, _) in processes.items()
                    if parent in descendants and pid not in descendants}
        if not children:
            break
        descendants.update(children)
        targets.update({pid: 'CodexNative' for pid in children})
    return targets


def sample_target(directory, round_number, pid, role):
    name = f'{round_number:03d}-{role}-{pid}.txt'
    started = datetime.datetime.now(datetime.timezone.utc).isoformat()
    try:
        result = subprocess.run(
            ['/usr/bin/sample', str(pid), '1', '10', '-file', str(directory / name)],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=10)
        return {'startedAt': started, 'pid': pid, 'role': role, 'file': name,
                'exitCode': result.returncode,
                'error': result.stdout[-1200:] if result.returncode else None}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {'startedAt': started, 'pid': pid, 'role': role,
                'file': name, 'error': type(error).__name__}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--duration', type=int, default=45,
                        help='Sampling window in seconds (default: 45, maximum: 120)')
    parser.add_argument('--output', type=Path, default=Path.home() / 'Downloads')
    parser.add_argument('--host-pid', type=int, help='Select one running Host PID when multiple Apps are open')
    parser.add_argument('--list', action='store_true', help='List Host candidates and Codex Providers without sampling')
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        parser.error('This tool requires macOS')
    if not 1 <= args.duration <= 120:
        parser.error('--duration must be between 1 and 120')
    process_text = subprocess.check_output(['ps', '-ww', '-axo', 'pid=,ppid=,comm='], text=True)
    candidates = host_candidates(process_text)
    if args.list:
        for pid, (parent, command) in parse_processes(process_text).items():
            if pid in candidates or Path(command).name == 'codepet-provider-codex':
                print(f'PID={pid} PPID={parent} {command}')
        if not candidates:
            print('没有发现 Host；请保持 Code Pet 运行，或用 --host-pid 指定实际 Host PID。')
        return
    if args.host_pid is None and len(candidates) > 1:
        parser.error('Multiple Hosts found; select one with --host-pid. Candidates: ' +
                     '; '.join(f'{pid}: {command}' for pid, command in candidates.items()))
    initial = discover_targets(process_text, args.host_pid)
    if 'Host' not in initial.values():
        parser.error('Running Code Pet Host not found. Installation path is unrestricted; '
                     'keep the App running, use --list, or select its PID with --host-pid.')
    if 'Provider' not in initial.values():
        parser.error('The running Codex Provider was not found; leave Code Pet and Codex enabled')
    selected_host = next(pid for pid, role in initial.items() if role == 'Host')
    print(f'Host PID={selected_host} {parse_processes(process_text)[selected_host][1]}', flush=True)
    print('开始采样：请现在在手机上刷新 Codex 最近对话，保持原 App 运行。', flush=True)
    print('仅采集函数栈；不重启进程、不发送任务、不读取数据库或认证文件。', flush=True)
    started = time.monotonic()
    records = []
    with tempfile.TemporaryDirectory(prefix='codepet-recent-live-') as temporary:
        directory = Path(temporary)
        round_number = 0
        with ThreadPoolExecutor(max_workers=4) as executor:
            while time.monotonic() - started < args.duration:
                text = subprocess.check_output(['ps', '-ww', '-axo', 'pid=,ppid=,comm='], text=True)
                targets = discover_targets(text, selected_host)
                jobs = [executor.submit(sample_target, directory, round_number, pid, role)
                        for pid, role in targets.items()]
                records.extend(job.result() for job in jobs)
                round_number += 1
                if not targets:
                    break
        report = {'schemaVersion': 1, 'durationSeconds': round(time.monotonic() - started, 3),
                  'kind': 'running-process-stacks', 'samples': records}
        (directory / 'index.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))
        args.output.mkdir(parents=True, exist_ok=True)
        output = args.output / ('codepet-recent-live-' + datetime.datetime.now().strftime('%Y%m%d-%H%M%S') + '.zip')
        with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED) as archive:
            for path in sorted(directory.iterdir()):
                archive.write(path, path.name)
        failures = sum(bool(row.get('error')) for row in records)
        print(f'采样完成：{len(records)} 份，失败 {failures} 份。')
        print(f'Result: {output}')
        if failures:
            print('若 sample 权限被拒绝，原因保存在 index.json；不要修改系统安全设置。')


if __name__ == '__main__':
    main()
