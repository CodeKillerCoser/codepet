"""Exercise the installed stable OpenCode plugin with isolated config and a local fake model.

Run: python3 scripts/test_opencode_observation.py --executable /opt/homebrew/bin/opencode
No model credentials or user configuration are used. Servers are always terminated.
"""
import argparse
import base64
import http.server
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tempfile
import threading
import time
import urllib.request


def wait_for(predicate, description, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.02)
    raise AssertionError('Timed out: ' + description)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', default='opencode')
    parser.add_argument('--remote-turns', action='store_true', help='Exercise Provider remote sends instead of observation checks')
    args = parser.parse_args()
    version = subprocess.check_output([args.executable, '--version'], text=True).strip()
    assert version == '1.18.25', 'This release fixture is verified for 1.18.25, got ' + version
    received = []
    model_calls = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            if self.path == '/event':
                assert self.headers.get('Authorization') == 'smoke-only'
                received.append(body)
                self.send_response(204)
                self.end_headers()
                return
            assert self.path == '/v1/chat/completions', self.path
            model_calls.append(body)
            messages = body.get('messages', [])
            permission = 'permission-smoke' in json.dumps(messages) and not any(m['role'] == 'tool' for m in messages)
            remote = 'remote-smoke' in json.dumps(messages)
            permission = permission or (remote and messages[-1]['role'] == 'user')
            delta = {'role': 'assistant', 'content': 'Smoke complete'}
            finish = 'stop'
            if permission:
                delta = {'role': 'assistant', 'tool_calls': [{'index': 0, 'id': 'call_smoke', 'type': 'function',
                    'function': {'name': 'bash', 'arguments': json.dumps({'command': 'echo codepet-smoke', 'description': 'Smoke test'})}}]}
                finish = 'tool_calls'
            self.send_response(200)
            if body.get('stream'):
                self.send_header('Content-Type', 'text/event-stream')
                self.end_headers()
                for data, reason in [(delta, None), ({}, finish)]:
                    chunk = {'id': 'chatcmpl-smoke', 'object': 'chat.completion.chunk', 'created': int(time.time()),
                        'model': 'smoke', 'choices': [{'index': 0, 'delta': data, 'finish_reason': reason}]}
                    self.wfile.write(('data: ' + json.dumps(chunk) + '\n\n').encode())
                self.wfile.write(b'data: [DONE]\n\n')
            else:
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(json.dumps({'id': 'chatcmpl-smoke', 'object': 'chat.completion', 'created': int(time.time()),
                    'model': 'smoke', 'choices': [{'index': 0, 'message': delta, 'finish_reason': finish}],
                    'usage': {'prompt_tokens': 1, 'completion_tokens': 1, 'total_tokens': 2}}).encode())

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    children = []
    logs = []
    try:
        with tempfile.TemporaryDirectory(prefix='codepet-opencode-stable-') as temporary:
            root = Path(temporary)
            config = root / 'config' / 'opencode'
            (config / 'plugins').mkdir(parents=True)
            (config / 'codepet-observation').mkdir()
            source = Path(__file__).resolve().parents[1] / 'crates/providers/codepet-provider-opencode/src/observation-plugin.ts'
            (config / 'plugins/codepet-observation.ts').write_bytes(source.read_bytes())
            (config / 'codepet-observation/endpoint.json').write_text(json.dumps({
                'url': 'http://127.0.0.1:%d/event' % server.server_port, 'token': 'smoke-only'}))
            (config / 'opencode.json').write_text(json.dumps({
                'model': 'codepet-smoke/smoke', 'small_model': 'codepet-smoke/smoke',
                'permission': {'bash': 'ask'},
                'provider': {'codepet-smoke': {'npm': '@ai-sdk/openai-compatible', 'name': 'Local fixture',
                    'options': {'baseURL': 'http://127.0.0.1:%d/v1' % server.server_port, 'apiKey': 'smoke-only'},
                    'models': {'smoke': {'name': 'Smoke', 'limit': {'context': 8192, 'output': 128}}}}}}))
            env = dict(os.environ)
            for key in ['OPENCODE_CONFIG', 'OPENCODE_CONFIG_CONTENT', 'OPENCODE_CONFIG_DIR']:
                env.pop(key, None)
            for kind in ['CONFIG', 'DATA', 'CACHE', 'STATE']:
                env['XDG_%s_HOME' % kind] = str(root / kind.lower())
            env.update(OPENCODE_SERVER_PASSWORD='smoke-only', OPENCODE_DISABLE_AUTOUPDATE='true')

            def start(index):
                workspace = root / ('workspace-%d' % index)
                workspace.mkdir()
                log_path = root / ('server-%d.log' % index)
                log = log_path.open('w+')
                logs.append(log)
                process = subprocess.Popen([args.executable, 'serve', '--hostname', '127.0.0.1', '--port', '0', '--print-logs'],
                    env=env, cwd=workspace, stdout=log, stderr=log, start_new_session=True)
                children.append(process)

                def address():
                    text = log_path.read_text()
                    if process.poll() is not None:
                        raise AssertionError(text[-3000:])
                    return re.search(r'opencode server listening on (http://127\.0\.0\.1:\d+)', text)

                base = wait_for(address, 'stable server startup').group(1)

                def request(route, data=None):
                    url = base + route + '?directory=' + urllib.parse.quote(str(workspace), safe='')
                    req = urllib.request.Request(url, data=None if data is None else json.dumps(data).encode(), headers={
                        'Authorization': 'Basic ' + base64.b64encode(b'opencode:smoke-only').decode(), 'Content-Type': 'application/json'})
                    with urllib.request.urlopen(req, timeout=20) as response:
                        raw = response.read()
                        return json.loads(raw) if raw else None

                # These are the public stable routes that initialize the normal CLI plugin hooks.
                request('/agent')
                return request

            def events(session_id):
                return [e for e in received if e['payload'].get('properties', {}).get('sessionID') == session_id]

            def has(session_id, event_type):
                return next((e for e in events(session_id) if e['payload']['type'] == event_type), None)

            if args.remote_turns:
                workspace = root / 'remote-workspace'
                workspace.mkdir()
                env['CODEPET_OPENCODE_EXECUTABLE'] = str(Path(shutil.which(args.executable) or args.executable).resolve())
                env['CODEPET_OPENCODE_TEST_WORKSPACE'] = str(workspace)
                subprocess.run(['cargo', 'test', '--manifest-path', str(source.parents[3] / 'Cargo.toml'),
                    '-p', 'codepet-provider-opencode', '--test', 'native_turns', '--', '--ignored', '--nocapture'],
                    cwd=source.parents[4], env=env, check=True)
                print('Native remote sends passed; local model requests:', len(model_calls))
                return

            first = start(1)
            success = first('/session', {'title': 'Stable success'})['id']
            first('/session/' + success + '/prompt_async', {'parts': [{'type': 'text', 'text': 'success-smoke'}]})
            wait_for(lambda: has(success, 'session.idle'), 'successful task completion')
            assert not has(success, 'session.error'), events(success)
            assert any(e['payload'].get('properties', {}).get('status', {}).get('type') == 'busy' for e in events(success))
            assert all(e['payload']['session']['title'] == 'Stable success' for e in events(success))

            permission = first('/session', {'title': 'Stable permission'})['id']
            first('/session/' + permission + '/prompt_async', {'parts': [{'type': 'text', 'text': 'permission-smoke'}]})
            asked = wait_for(lambda: has(permission, 'permission.asked'), 'permission hook')
            first('/permission/' + asked['payload']['properties']['id'] + '/reply', {'reply': 'reject'})
            wait_for(lambda: has(permission, 'permission.replied'), 'permission reply notification')
            wait_for(lambda: has(permission, 'session.idle'), 'task after rejected permission')

            # An independent process using the same global plugin must also report activity.
            second = start(2)
            failed = second('/session', {'title': 'Stable failure'})['id']
            second('/session/' + failed + '/prompt_async', {'model': {'providerID': 'codepet-missing', 'modelID': 'none'},
                'parts': [{'type': 'text', 'text': 'failure-smoke'}]})
            wait_for(lambda: has(failed, 'session.error'), 'failed task event from second process')
            wait_for(lambda: has(failed, 'session.idle'), 'idle after failure')
            assert has(success, 'session.created')['payload']['cwd'] != has(failed, 'session.created')['payload']['cwd']
            assert model_calls, 'No request reached the local model fixture'
            assert not any(e['payload']['type'].startswith(('message.', 'plugin.', 'catalog.')) for e in received)
            print(json.dumps({'version': version, 'independentProcesses': len(children), 'localModelCalls': len(model_calls),
                'success': [e['payload']['type'] for e in events(success)],
                'permission': [e['payload']['type'] for e in events(permission)],
                'failure': [e['payload']['type'] for e in events(failed)]}, indent=2))
            # Stop children before their isolated config directory is removed.
            for process in children:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=4)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
    except Exception:
        for index, log in enumerate(logs):
            log.seek(0)
            print('Isolated OpenCode server %d log:\n%s' % (index + 1, log.read()[-8000:]))
        raise
    finally:
        for process in children:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        for log in logs:
            log.close()
        server.shutdown()
        server.server_close()


if __name__ == '__main__':
    main()
