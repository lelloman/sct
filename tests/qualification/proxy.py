"""Disposable TCP fault proxy. Control port belongs only to the test network."""
import asyncio
import json
import os

mode = 'normal'
connections = set()
stats = {'accepted': 0, 'upstream_bytes': 0, 'downstream_bytes': 0}

async def relay(reader, writer, downstream):
    transferred = 0
    while data := await reader.read(65536):
        if mode == 'offline':
            return
        while mode == 'stall':
            await asyncio.sleep(.05)
        if mode == 'latency':
            await asyncio.sleep(.025)
        if mode == 'cut' and downstream and transferred >= 32768:
            raise ConnectionResetError('injected truncation')
        writer.write(data)
        await writer.drain()
        transferred += len(data)
        stats['downstream_bytes' if downstream else 'upstream_bytes'] += len(data)
    # A TCP FIN closes one direction only. Preserve a response still in flight
    # when the HTTP client finishes/closes its request side.
    if writer.can_write_eof():
        writer.write_eof()
        await writer.drain()

async def proxy(reader, writer):
    stats['accepted'] += 1
    connections.add(writer)
    upstream = None
    tasks = []
    try:
        if mode == 'offline':
            return
        other, upstream = await asyncio.open_connection(os.environ['UPSTREAM_HOST'], int(os.environ.get('UPSTREAM_PORT', '9000')))
        connections.add(upstream)
        tasks = [asyncio.create_task(relay(reader, upstream, False)),
                 asyncio.create_task(relay(other, writer, True))]
        await asyncio.gather(*tasks)
    except (ConnectionError, OSError):
        pass
    finally:
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        for stream in (writer, upstream):
            if stream:
                connections.discard(stream)
                stream.close()
                try:
                    await stream.wait_closed()
                except (ConnectionError, OSError):
                    pass

async def control(reader, writer):
    global mode
    status = '200 OK'
    try:
        header = await asyncio.wait_for(reader.readuntil(b'\r\n\r\n'), 3)
        lines = header.decode().split('\r\n')
        if lines[0].startswith('POST /mode '):
            length = next(int(l.split(':', 1)[1]) for l in lines if l.lower().startswith('content-length:'))
            if not 0 < length < 1024:
                raise ValueError('invalid length')
            value = json.loads(await reader.readexactly(length))['mode']
            if value not in ('normal', 'offline', 'stall', 'latency', 'cut'):
                raise ValueError('invalid mode')
            mode = value
            # Discard existing pooled connections, so every fault is effective.
            for stream in list(connections):
                stream.close()
        elif not lines[0].startswith('GET /state '):
            raise ValueError('unknown operation')
        body = json.dumps({'mode': mode, **stats}).encode()
    except (ValueError, KeyError, StopIteration, asyncio.IncompleteReadError, TimeoutError):
        status, body = '400 Bad Request', b'{}'
    writer.write(f'HTTP/1.1 {status}\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n'.encode() + body)
    await writer.drain()
    writer.close()
    await writer.wait_closed()

async def main():
    server = await asyncio.start_server(proxy, os.environ.get('LISTEN_HOST', '0.0.0.0'), int(os.environ.get('LISTEN_PORT', '9001')))
    admin = await asyncio.start_server(control, os.environ.get('LISTEN_HOST', '0.0.0.0'), int(os.environ.get('CONTROL_PORT', '8474')))
    async with server, admin:
        await asyncio.gather(server.serve_forever(), admin.serve_forever())

asyncio.run(main())
