#!/usr/bin/env python3
"""Real-process shutdown checks (disposable Postgres); build sct-server/sct-recover first."""
import json
import os
import re
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
BINARY = Path(os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target'))) / 'debug/sct-server'


def command(*args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def run(env, number, sig, lose_lease=None, drain=False):
    with tempfile.TemporaryFile(mode='w+') as log:
        process = subprocess.Popen([str(BINARY)], cwd=ROOT, env=env, stdout=log, stderr=log)
        try:
            for _ in range(200):
                if process.poll() is not None:
                    log.seek(0)
                    raise AssertionError(log.read())
                try:
                    urllib.request.urlopen(f'http://127.0.0.1:{number}/', timeout=.2).close()
                    break
                except OSError:
                    time.sleep(.05)
            else:
                raise AssertionError('server did not listen')
            if drain:
                connection = socket.create_connection(('127.0.0.1', number))
                connection.settimeout(10)
                connection.sendall(b'GET /large.bin HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n')
                data = connection.recv(4096)
                assert b'200 OK' in data
            if lose_lease:
                lose_lease()
            else:
                process.send_signal(sig)
            if drain:
                while chunk := connection.recv(65536):
                    data += chunk
                connection.close()
                assert len(data.split(b'\r\n\r\n', 1)[1]) == 8 * 1024 * 1024
            assert process.wait(timeout=20) == (1 if lose_lease else 0)
            log.seek(0)
            output = log.read()
            records = []
            for line in output.splitlines():
                try:
                    records.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
            if lose_lease:
                assert any(record.get('event') == 'fatal' for record in records), output
                assert 'writer-lease' in output, output
                assert not any(record.get('event') == 'shutdown_complete' for record in records), output
            else:
                assert any(record.get('event') == 'shutdown_complete' for record in records), output
            with socket.socket() as probe:
                assert probe.connect_ex(('127.0.0.1', number)) != 0
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()


with tempfile.TemporaryDirectory(prefix='sct-lifecycle-') as directory:
    fixture = Path(directory)
    (fixture / 'index.html').write_text('lifecycle')
    (fixture / 'large.bin').write_bytes(b'x' * (8 * 1024 * 1024))
    env = {k: v for k, v in os.environ.items() if not k.startswith('SCT_')}
    number = port()
    env.update(SCT_BIND=f'127.0.0.1:{number}', SCT_WEB_DIR=directory)
    for sig in (signal.SIGINT, signal.SIGTERM):
        run(env, number, sig, drain=True)
    container = command('docker', 'run', '-d', '--rm', '-e', 'POSTGRES_PASSWORD=sct-test', '-e', 'POSTGRES_DB=sct_test', '-p', '127.0.0.1::5432', 'postgres:17-alpine')
    try:
        for _ in range(60):
            if subprocess.run(['docker', 'exec', container, 'pg_isready', '-h', '127.0.0.1', '-U', 'postgres'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
                break
            time.sleep(.5)
        address = command('docker', 'port', container, '5432/tcp')
        env.update(SCT_TEST_DATABASE_URL=f'postgres://postgres:sct-test@{address}/sct_test', SCT_E2E_PORT=str(number), SCT_TEST_RECOVERY_BINARY=str(BINARY.parent / 'sct-recover'))
        subprocess.run(['cargo', 'run', '--locked', '--quiet', '-p', 'sct-server', '--example', 'foundation-fixture', '--', directory], cwd=ROOT, env=env, check=True)
        # This suite needs only its temporary static fixture, not a frontend build.
        config_path = fixture / 'config.toml'
        config, replaced = re.subn(r'^web_dir = .*$', 'web_dir = ' + json.dumps(directory), config_path.read_text(), flags=re.MULTILINE)
        assert replaced == 1
        config_path.write_text(config)
        details = json.loads((fixture / 'fixture.json').read_text())
        env.update(SCT_CONFIG=str(fixture / 'config.toml'), SCT_E2E_DATABASE_URL=details['database_url'])
        # Immediate restart verifies that the previous process released its writer lease.
        for sig in (signal.SIGTERM, signal.SIGINT):
            run(env, number, sig)
        def lose_lease():
            command('docker', 'exec', container, 'psql', '-U', 'postgres', '-d', 'sct_test', '-c', f"UPDATE {details['schema']}.server_lease SET owner='lost' WHERE singleton")
        run(env, number, signal.SIGTERM, lose_lease=lose_lease)
    finally:
        command('docker', 'rm', '-f', container)
print('PASS: SIGINT/SIGTERM HTTP drain, writer release/restart, lease loss exits nonzero')
