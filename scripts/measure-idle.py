#!/usr/bin/env python3
"""Measure only the isolated qualification daemon, never existing services."""
import json
import os
import pathlib
import subprocess
import tempfile
import time

binary = pathlib.Path('target/release/apollo-artifactd').resolve()
with tempfile.TemporaryDirectory(prefix='artifactd-idle-') as directory, \
        tempfile.TemporaryDirectory(prefix='aid-', dir='/tmp') as runtime_directory:
    root = pathlib.Path(directory)
    store, runtime = root / 'store', pathlib.Path(runtime_directory)
    store.mkdir(mode=0o700)
    process = subprocess.Popen([str(binary), '--store', str(store), '--socket',
                                str(runtime / 'api.sock')], stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL)
    try:
        deadline = time.monotonic() + 10
        while not (runtime / 'api.sock').exists():
            if process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError('qualification daemon did not start')
            time.sleep(0.01)
        def snapshot():
            proc = pathlib.Path('/proc') / str(process.pid)
            fields = (proc / 'stat').read_text().split()
            status = {line.split(':', 1)[0]: line.split(':', 1)[1].strip()
                      for line in (proc / 'status').read_text().splitlines()
                      if line.startswith(('VmRSS:', 'VmHWM:', 'Threads:'))}
            return {'status': status, 'fds': len(list((proc / 'fd').iterdir())),
                    'cpu_ticks': int(fields[13]) + int(fields[14])}
        first = snapshot()
        start = time.monotonic()
        time.sleep(5)
        last = snapshot()
        elapsed = time.monotonic() - start
        print(json.dumps({'baseline': first, 'after_idle': last,
                          'elapsed_seconds': elapsed,
                          'cpu_seconds': (last['cpu_ticks'] - first['cpu_ticks']) /
                          os.sysconf('SC_CLK_TCK')}))
    finally:
        process.kill()
        process.wait()
