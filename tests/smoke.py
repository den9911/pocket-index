import json, os, pathlib, subprocess, tempfile, time, urllib.parse, urllib.request
binary = pathlib.Path('target/release/pocket-index').resolve()
def request(url, data=None):
    req = urllib.request.Request('http://127.0.0.1:3002'+url, data=data, headers={'Content-Type':'application/json','X-Pocket-Request':'1'} if data else {})
    with urllib.request.urlopen(req, timeout=5) as r: return r.read()
def wait():
    for _ in range(100):
        try: return request('/api/status')
        except OSError: time.sleep(.1)
    raise RuntimeError('server did not start')
with tempfile.TemporaryDirectory() as directory:
    root=pathlib.Path(directory); notes=root/'notes'; notes.mkdir(); (notes/'note.md').write_text('Rust привет unique',encoding='utf8')
    env={**os.environ,'POCKET_INDEX_DB':str(root/'index.sqlite')}
    p=subprocess.Popen([str(binary)],cwd=root,env=env,stdout=subprocess.DEVNULL)
    try:
        wait(); request('/api/index',json.dumps({'path':str(notes)}).encode())
        for _ in range(100):
            status=json.loads(request('/api/status'))
            if status['phase']!='running': break
            time.sleep(.1)
        assert status['phase']=='complete',status
        assert json.loads(request('/api/search?q='+urllib.parse.quote('привет')))[0]['title']=='note.md'
    finally: p.terminate(); p.wait(timeout=5)
    p=subprocess.Popen([str(binary)],cwd=root,env=env,stdout=subprocess.DEVNULL)
    try:
        wait(); assert json.loads(request('/api/search?q=Rust'))[0]['title']=='note.md'
    finally: p.terminate(); p.wait(timeout=5)
print('PASS: index, Russian search and restart persistence via HTTP')
