"""Exercise the real server and on-disk SQLite snapshots, including interrupted rebuild."""
import json
import os
import pathlib
import signal
import sqlite3
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request

binary = pathlib.Path('target/release/pocket-index').resolve()


def request(path, data=None):
    headers = {'Content-Type': 'application/json', 'X-Pocket-Request': '1'} if data is not None else {}
    body = json.dumps(data).encode() if data is not None else None
    req = urllib.request.Request('http://127.0.0.1:3002' + path, data=body, headers=headers)
    with urllib.request.urlopen(req, timeout=5) as response:
        payload = response.read()
        return (json.loads(payload) if payload else None), response.headers


def start(root, db):
    process = subprocess.Popen([str(binary)], cwd=root,
                               env={**os.environ, 'POCKET_INDEX_DB': str(db)},
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(200):
        if process.poll() is not None:
            raise RuntimeError('Server exited before readiness')
        try:
            request('/api/status')
            return process
        except OSError:
            time.sleep(.025)
    process.kill()
    process.wait()
    raise RuntimeError('Server did not start')


def rebuild(folder):
    request('/api/index', {'path': str(folder)})
    for _ in range(400):
        status, _ = request('/api/status')
        if status['phase'] != 'running':
            return status
        time.sleep(.025)
    raise RuntimeError('Rebuild did not finish')


def hits(term):
    return request('/api/search?q=' + urllib.parse.quote(term))


with tempfile.TemporaryDirectory(prefix='pocket snapshot ') as directory:
    root = pathlib.Path(directory)
    notes = root / 'notes'
    notes.mkdir()
    db = root / 'index.sqlite'
    (notes / 'one.md').write_text('oldtoken Прежняя заметка', encoding='utf8')
    (notes / 'two.txt').write_text('oldtoken Second note', encoding='utf8')
    process = start(root, db)
    try:
        status = rebuild(notes)
        assert status['phase'] == 'complete', status
        original = status['active']
        assert original == {'generation': 1, 'documents': 2}, status
        result, headers = hits('oldtoken')
        assert len(result) == 2 and headers['x-pocket-index-generation'] == '1'

        (notes / 'one.md').write_text('newtoken Changed note', encoding='utf8')
        (notes / 'bad.txt').write_bytes(b'\xff\xfe')
        status = rebuild(notes)
        assert status['phase'] == 'retained' and status['reason'] == 'read_errors', status
        assert status['active'] == original and status['errors'] == 1, status
        assert len(hits('oldtoken')[0]) == 2 and not hits('newtoken')[0]

        (notes / 'bad.txt').unlink()
        blocked = notes / 'blocked'
        blocked.mkdir()
        (blocked / 'private.md').write_text('do not index partially', encoding='utf8')
        blocked.chmod(0)
        try:
            if os.geteuid() != 0:
                status = rebuild(notes)
                assert status['phase'] == 'retained' and status['errors'] > 0, status
                assert status['active'] == original
        finally:
            blocked.chmod(0o700)
            (blocked / 'private.md').unlink()
            blocked.rmdir()

        (notes / 'large.md').write_bytes(b'x' * (1_048_576 + 1))
        status = rebuild(notes)
        assert status['phase'] == 'retained' and status['active'] == original, status
        (notes / 'large.md').unlink()

        status = rebuild(root / 'not-a-real-private-folder')
        assert status['phase'] == 'error' and status['reason'] == 'source_unavailable', status
        assert status['active'] == original
        assert 'private-folder' not in json.dumps(status) and 'newtoken' not in json.dumps(status)

        with sqlite3.connect(db) as connection:
            connection.executescript("CREATE TRIGGER reject_snapshot BEFORE UPDATE ON pocket_index_state BEGIN SELECT RAISE(ABORT,'private fixture must not leak'); END;")
        status = rebuild(notes)
        assert status['phase'] == 'error' and status['reason'] == 'storage', status
        assert status['active'] == original and status['message'] == ''
        assert len(hits('oldtoken')[0]) == 2
        with sqlite3.connect(db) as connection:
            connection.execute('DROP TRIGGER reject_snapshot')

        # A real SIGKILL during an open write transaction must not publish the partial FTS tree.
        slow = root / 'interrupted'
        slow.mkdir()
        content = ('replacementtoken temporary scan ' * 1057)[:32768]
        for i in range(2000):
            (slow / f'{i:04d}.md').write_text(content, encoding='utf8')
        request('/api/index', {'path': str(slow)})
        for _ in range(1000):
            status, _ = request('/api/status')
            if status['phase'] == 'running' and 0 < status['processed'] < 1000:
                os.kill(process.pid, signal.SIGSTOP)
                process.kill()
                process.wait(timeout=5)
                break
            if status['phase'] != 'running':
                raise AssertionError('Interruption fixture completed before the stop point')
            time.sleep(.002)
        else:
            raise AssertionError('Did not reach interruption point')

        process = start(root, db)
        status, _ = request('/api/status')
        assert status['phase'] == 'idle' and status['active'] == original, status
        assert len(hits('oldtoken')[0]) == 2 and not hits('replacementtoken')[0]

        status = rebuild(notes)
        assert status['phase'] == 'complete' and status['active']['generation'] == 2, status
        assert status['active']['documents'] == 2
        assert len(hits('newtoken')[0]) == 1
        status = rebuild(notes)
        assert status['phase'] == 'complete' and status['active']['documents'] == 2
        assert len(hits('newtoken')[0]) == 1, 'retry duplicated a document'
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)

print('PASS HTTP snapshots: partial/read/size/SQL/root failures, real SIGKILL recovery, versions and safe retry')
