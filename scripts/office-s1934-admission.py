#!/usr/bin/env python3
"""Loopback-only event-driven S1934 admission prototype and sustained trials.

Local signed sessions/transactions are harness identities, not an OIDC provider.
Never qualifies tenant sign-in, routed NAT/VPN/VDI or installed A1 searches.
"""
import argparse
import asyncio
from collections import Counter, OrderedDict
import hashlib
import hmac
import ipaddress
import resource
import threading
import json
import os
from pathlib import Path
import secrets
import socket
import ssl
import subprocess
import time

import h2.config
import h2.connection
import h2.events
import h2.settings

RTK = '/opt/homebrew/bin/rtk'


def source_key(address):
    ip = ipaddress.ip_address(address.split('%')[0])
    return str(ipaddress.ip_network(str(ip) + '/64', strict=False)) if ip.version == 6 else str(ip)


class Bucket:
    def __init__(self, rate, burst, now):
        self.rate, self.burst, self.tokens, self.at = rate, burst, float(burst), now

    def take(self, now):
        self.tokens = min(self.burst, self.tokens + max(0, now-self.at)*self.rate)
        self.at = now
        if self.tokens < 1:
            return False
        self.tokens -= 1
        return True


class Registry:
    def __init__(self):
        self.sources = OrderedDict()
        self.global_bucket = Bucket(100, 100, time.monotonic())
        self.entries = {}
        self.queue = OrderedDict()
        self.metrics = Counter()
        self.people = Counter()
        self.sessions = {}
        self.transactions = {}
        self.secret = secrets.token_bytes(32)
        self.auth_cpu = asyncio.Semaphore(150)
        self.signin_cpu = asyncio.Semaphore(10)
        self.number = 0

    def token(self, person, session):
        value = f'{person}.{session}'
        signature = hmac.new(self.secret, value.encode(), hashlib.sha256).hexdigest()
        token = value + '.' + signature
        self.sessions[token] = (str(person), time.monotonic()+3600)
        return token

    def verified(self, token):
        row = self.sessions.get(token)
        return row[0] if row and row[1] > time.monotonic() else None

    def gate(self, address, now=None):
        now = time.monotonic() if now is None else now
        key = source_key(address)
        self.metrics['arrivals'] += 1
        if key not in self.sources:
            # Never reset a live attacker's bucket when the table fills.
            for old, row in list(self.sources.items()):
                if now-row['last'] > 60 and not any(e['key'] == old for e in self.entries.values()):
                    del self.sources[old]
            if len(self.sources) >= 1024:
                self.metrics['source_table_rejected'] += 1
                return None
            self.sources[key] = {'tls': Bucket(10, 50, now), 'start': Bucket(1, 25, now), 'last': now}
        row = self.sources[key]
        row['last'] = now
        if not row['tls'].take(now) or not self.global_bucket.take(now):
            self.metrics['bucket_rejected'] += 1
            return None
        pending = [e for e in self.entries.values() if e['class'] == 'pending']
        if len(self.entries) >= 200 or len(pending) >= 40 or sum(e['key'] == key for e in pending) >= 8:
            self.metrics['capacity_rejected'] += 1
            return None
        if key in self.queue:
            self.metrics['queued_source_rejected'] += 1
            return None
        self.number += 1
        self.entries[self.number] = {'key': key, 'class': 'pending', 'at': now, 'person': None}
        self.metrics['admitted_before_tls'] += 1
        self.metrics['peak_connections'] = max(self.metrics['peak_connections'], len(self.entries))
        self.metrics['peak_pending'] = max(self.metrics['peak_pending'], len(pending)+1)
        self.metrics['peak_source_table'] = max(self.metrics['peak_source_table'], len(self.sources))
        return self.number

    def authenticate(self, number, token):
        entry = self.entries[number]
        person = self.verified(token)
        if person is None:
            return False
        if entry['class'] == 'auth':
            return entry['person'] == person
        if self.people[person] >= 6 or sum(self.people.values()) >= 150:
            self.metrics['person_capacity_rejected'] += 1
            return False
        entry['class'], entry['person'] = 'auth', person
        self.people[person] += 1
        self.metrics['auth_progress'] += 1
        self.metrics['peak_authenticated'] = max(self.metrics['peak_authenticated'], sum(self.people.values()))
        return True

    async def response(self, number, headers):
        entry = self.entries[number]
        cookie = headers.get('cookie', '')
        token = cookie.removeprefix('session=')
        if entry['class'] == 'auth' and self.verified(token) != entry['person']:
            self.people[entry['person']] -= 1
            entry['class'], entry['person'] = 'pending', None
            self.metrics['revocation_demotions'] += 1
        path = headers.get(':path', '/')
        if path.startswith('/auth/start'):
            row = self.sources[entry['key']]
            now = time.monotonic()
            self.transactions = {k: v for k, v in self.transactions.items() if v['expires'] > now}
            if not row['start'].take(now) or len(self.transactions) >= 100:
                self.metrics['signin_start_rejected'] += 1
                return 429, b'rejected', []
            async with self.signin_cpu:
                state = secrets.token_hex(16)
                bound = secrets.token_hex(16)
                self.transactions[state] = {'cookie': bound, 'expires': now+300}
                self.metrics['signin_starts'] += 1
                return 200, state.encode(), [('set-cookie', 'transaction=' + bound)]
        if path.startswith('/auth/callback?state='):
            state = path.split('=', 1)[1]
            transaction = self.transactions.get(state)
            if not transaction or transaction['cookie'] != cookie.removeprefix('transaction=') or transaction['expires'] <= time.monotonic():
                return 401, b'unbound', []
            if sum(e['class'] == 'signin' for e in self.entries.values()) >= 10:
                return 503, b'capacity', []
            entry['class'] = 'signin'
            async with self.signin_cpu:
                del self.transactions[state]
                self.metrics['signin_callbacks'] += 1
                return 200, b'local-bound-callback', []
        if self.authenticate(number, token):
            async with self.auth_cpu:
                return 200, b'loopback-admission-response', []
        return 401, b'unauthorized', []


class Server:
    def __init__(self, context):
        self.context, self.registry = context, Registry()
        self.tasks = set()
        self.listener = socket.socket()
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(256)
        self.listener.setblocking(False)
        self.port = self.listener.getsockname()[1]

    async def accept(self):
        loop = asyncio.get_running_loop()
        while True:
            raw, address = await loop.sock_accept(self.listener)
            raw.setblocking(False)
            number = self.registry.gate(address[0])
            if number is None:
                raw.close()
                continue
            key = self.registry.entries[number]['key']
            self.registry.queue[key] = (raw, number)
            # One turn/source; all entries, including queued/draining, counted.
            _, (raw, number) = self.registry.queue.popitem(last=False)
            task = asyncio.create_task(self.connection(raw, number))
            self.tasks.add(task)
            task.add_done_callback(self.tasks.discard)

    async def connection(self, raw, number):
        registry = self.registry
        writer = None
        try:
            loop = asyncio.get_running_loop()
            reader = asyncio.StreamReader(limit=16384)
            protocol = asyncio.StreamReaderProtocol(reader)
            transport, _ = await loop.connect_accepted_socket(lambda: protocol, raw, ssl=self.context, ssl_handshake_timeout=5)
            writer = asyncio.StreamWriter(transport, protocol, reader, loop)
            ssl_object = writer.get_extra_info('ssl_object')
            if ssl_object.selected_alpn_protocol() == 'h2':
                await self.h2(reader, writer, number)
            else:
                await self.h1(reader, writer, number)
        except (OSError, ValueError, asyncio.TimeoutError, asyncio.IncompleteReadError, asyncio.LimitOverrunError):
            registry.metrics['deadline_or_io_closures'] += 1
        except Exception as exc:
            registry.metrics['unexpected_' + type(exc).__name__] += 1
        finally:
            entry = registry.entries.get(number)
            if entry:
                entry['class_before_close'] = entry['class']
                entry['class'] = 'closing'
            registry.metrics['drains_started'] += 1
            if writer:
                writer.close()
                try:
                    await asyncio.wait_for(writer.wait_closed(), 1)
                except (OSError, asyncio.TimeoutError):
                    writer.transport.abort()
            raw.close()
            entry = registry.entries.pop(number, None)
            if entry and entry['person'] is not None:
                registry.people[entry['person']] -= 1
            registry.metrics['drains_completed'] += 1

    async def h1(self, reader, writer, number):
        while True:
            entry = self.registry.entries[number]
            # Authenticated idle connections survive; partial headers have an
            # absolute deadline from the first byte, never reset by dribbling.
            first = await reader.readexactly(1) if entry['class'] == 'auth' else await asyncio.wait_for(reader.readexactly(1), 5)
            header = first + await asyncio.wait_for(reader.readuntil(b'\r\n\r\n'), 5)
            if len(header) > 16384:
                raise ValueError('headers')
            lines = header.decode('latin1').split('\r\n')
            headers = {':path': lines[0].split()[1]}
            headers.update((line.split(':', 1)[0].lower(), line.split(':', 1)[1].strip()) for line in lines[1:] if ':' in line)
            size = int(headers.get('content-length', 0))
            if size > 16384:
                raise ValueError('body')
            if size:
                await asyncio.wait_for(reader.readexactly(size), 5)
            status, body, extra = await self.registry.response(number, headers)
            writer.write((f'HTTP/1.1 {status} Result\r\nContent-Length: {len(body)}\r\n' +
                          ''.join(k + ': ' + v + '\r\n' for k, v in extra) + '\r\n').encode() + body)
            await asyncio.wait_for(writer.drain(), 10)
            if self.registry.entries[number]['class'] == 'signin':
                return

    async def h2(self, reader, writer, number):
        connection = h2.connection.H2Connection(config=h2.config.H2Configuration(client_side=False, header_encoding='utf-8'))
        connection.initiate_connection()
        connection.update_settings({h2.settings.SettingCodes.MAX_CONCURRENT_STREAMS: 8, h2.settings.SettingCodes.MAX_HEADER_LIST_SIZE: 16384})
        writer.write(connection.data_to_send())
        streams = {}
        frame_buffer = bytearray()
        preface = 24
        header_started = None
        header_bytes = 0
        frame_started = None
        stream_deadlines = {}
        while True:
            entry = self.registry.entries[number]
            deadlines = list(stream_deadlines.values())
            if header_started is not None:
                deadlines.append(header_started+5)
            if frame_started is not None:
                deadlines.append(frame_started+5)
            if deadlines:
                data = await asyncio.wait_for(reader.read(16384), max(.001, min(deadlines)-time.monotonic()))
            elif entry['class'] == 'auth':
                data = await reader.read(16384)
            else:
                data = await asyncio.wait_for(reader.read(16384), 5)
            if not data:
                return
            now = time.monotonic()
            frame_buffer.extend(data)
            if frame_started is None:
                frame_started = now
            if preface:
                consumed = min(preface, len(frame_buffer))
                del frame_buffer[:consumed]
                preface -= consumed
            while not preface and len(frame_buffer) >= 9:
                length = int.from_bytes(frame_buffer[:3], 'big')
                frame_type, flags = frame_buffer[3], frame_buffer[4]
                if frame_type in (1, 9) and header_bytes+length > 16384:
                    raise ValueError('incomplete h2 header bytes')
                if len(frame_buffer) < 9+length:
                    break
                if frame_type == 1 and header_started is None:
                    header_started, header_bytes = frame_started, 0
                if frame_type in (1, 9):
                    header_bytes += length
                    if flags & 4:
                        header_started, header_bytes = None, 0
                del frame_buffer[:9+length]
                frame_started = now if frame_buffer else None
            if not frame_buffer and not preface:
                frame_started = None
            for event in connection.receive_data(data):
                if isinstance(event, h2.events.RequestReceived):
                    headers = dict(event.headers)
                    if len(streams) >= 8 or sum(len(k)+len(v) for k, v in event.headers) > 16384:
                        raise ValueError('h2 headers')
                    streams[event.stream_id] = [headers, 0]
                    stream_deadlines[event.stream_id] = time.monotonic()+5
                elif isinstance(event, h2.events.DataReceived):
                    streams[event.stream_id][1] += len(event.data)
                    if streams[event.stream_id][1] > 16384:
                        raise ValueError('h2 body')
                    connection.acknowledge_received_data(event.flow_controlled_length, event.stream_id)
                elif isinstance(event, h2.events.StreamEnded):
                    headers, _ = streams.pop(event.stream_id)
                    stream_deadlines.pop(event.stream_id, None)
                    status, body, extra = await self.registry.response(number, headers)
                    connection.send_headers(event.stream_id, [(':status', str(status)), *extra])
                    connection.send_data(event.stream_id, body, end_stream=True)
            writer.write(connection.data_to_send())
            await asyncio.wait_for(writer.drain(), 10)


class Client:
    def __init__(self, server, context, protocol, token=''):
        self.server, self.context, self.protocol, self.token = server, context, protocol, token
        self.reader = self.writer = self.h2 = None
        self.lock = asyncio.Lock()

    async def connect(self, source='127.0.0.1'):
        self.reader, self.writer = await asyncio.wait_for(asyncio.open_connection('127.0.0.1', self.server.port, ssl=self.context, server_hostname='localhost', local_addr=(source, 0)), 5)
        if self.protocol == 'h2':
            self.h2 = h2.connection.H2Connection(config=h2.config.H2Configuration(client_side=True, header_encoding='utf-8'))
            self.h2.initiate_connection()
            self.writer.write(self.h2.data_to_send())

    async def request(self, path='/', cookie=None):
        async with self.lock:
            cookie = cookie if cookie is not None else 'session=' + self.token
            if self.protocol == 'http/1.1':
                self.writer.write(f'GET {path} HTTP/1.1\r\nHost: localhost\r\nCookie: {cookie}\r\n\r\n'.encode())
                await self.writer.drain()
                header = await asyncio.wait_for(self.reader.readuntil(b'\r\n\r\n'), 5)
                lines = header.decode().split('\r\n')
                status = int(lines[0].split()[1])
                headers = {k.lower(): v.strip() for k, v in (line.split(':', 1) for line in lines[1:] if ':' in line)}
                body = await asyncio.wait_for(self.reader.readexactly(int(headers['content-length'])), 5)
                return status, headers, body
            stream = self.h2.get_next_available_stream_id()
            self.h2.send_headers(stream, [(':method', 'GET'), (':scheme', 'https'), (':authority', 'localhost'), (':path', path), ('cookie', cookie)], end_stream=True)
            self.writer.write(self.h2.data_to_send())
            await self.writer.drain()
            headers, body = {}, b''
            deadline = time.monotonic()+5
            while True:
                data = await asyncio.wait_for(self.reader.read(65536), max(.001, deadline-time.monotonic()))
                if not data:
                    raise OSError('h2 EOF')
                for event in self.h2.receive_data(data):
                    if isinstance(event, h2.events.ResponseReceived) and event.stream_id == stream:
                        headers = dict(event.headers)
                    elif isinstance(event, h2.events.DataReceived) and event.stream_id == stream:
                        body += event.data
                        self.h2.acknowledge_received_data(event.flow_controlled_length, stream)
                    elif isinstance(event, h2.events.StreamEnded) and event.stream_id == stream:
                        return int(headers[':status']), headers, body
                self.writer.write(self.h2.data_to_send())

    async def close(self):
        if self.writer:
            self.writer.close()
            try:
                await asyncio.wait_for(self.writer.wait_closed(), 1)
            except (OSError, asyncio.TimeoutError):
                self.writer.transport.abort()


async def trial(root, protocol, seconds, certificate, key):
    available_sources, unavailable_sources = [], {}
    for address in ['127.0.0.1', *[f'127.0.0.{n}' for n in range(2, 10)]]:
        raw = socket.socket()
        try:
            raw.bind((address, 0))
            available_sources.append(address)
        except OSError as exc:
            unavailable_sources[address] = exc.errno
        finally:
            raw.close()
    multi_sources = [s for s in available_sources if s != '127.0.0.1'] or ['127.0.0.1']
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certificate, key)
    context.set_alpn_protocols([protocol])
    client_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    client_context.load_verify_locations(certificate)
    client_context.set_alpn_protocols([protocol])
    server = Server(context)
    accept = asyncio.create_task(server.accept())
    sessions = []
    # Three browser sessions/person, two HTTP/1.1 connections/session = six.
    connections = 6 if protocol == 'http/1.1' else 3
    for person in range(25):
        for number in range(connections):
            token = server.registry.token(person, number//2 if protocol == 'http/1.1' else number)
            client = Client(server, client_context, protocol, token)
            await client.connect()
            status, _, _ = await client.request()
            if status != 200:
                raise RuntimeError('establishment failed')
            sessions.append((person, client))
            await asyncio.sleep(.11)
    results = []
    resource_samples = []
    stop = asyncio.Event()
    attack = Counter()

    attack_tasks = set()
    async def attack_connection(i):
        source = '127.0.0.1' if time.monotonic()-began < seconds/2 else multi_sources[i % len(multi_sources)]
        kind = ('raw_tls', 'partial_headers', 'completed_preauth')[i%3]
        attack[kind + '_offered'] += 1
        if kind == 'raw_tls':
            raw = socket.socket()
            raw.setblocking(False)
            try:
                raw.bind((source, 0))
                await asyncio.wait_for(asyncio.get_running_loop().sock_connect(raw, ('127.0.0.1', server.port)), .5)
                attack['tcp_connected'] += 1
                await asyncio.get_running_loop().sock_sendall(raw, b'\x16\x03\x03\x00\xff')
                await asyncio.sleep(5.1)
            except (OSError, asyncio.TimeoutError):
                attack['tcp_failed'] += 1
            finally:
                raw.close()
            return
        client = Client(server, client_context, protocol)
        try:
            await client.connect(source)
            attack[kind + '_tls_completed'] += 1
            if kind == 'completed_preauth':
                status, _, _ = await client.request()
                attack['preauth_responses_' + str(status)] += 1
                await asyncio.sleep(5.1)
            elif protocol == 'http/1.1':
                client.writer.write(b'GET / HTTP/1.1\r\nHost: localhost\r\nX-Slow: ')
                for _ in range(12):
                    client.writer.write(b'x')
                    await client.writer.drain()
                    await asyncio.sleep(.5)
            else:
                # Open a header block without END_HEADERS, then dribble
                # CONTINUATION fragments. Its absolute deadline must hold.
                from hyperframe.frame import HeadersFrame, ContinuationFrame
                frame = HeadersFrame(1)
                frame.data = b'\x82'
                client.writer.write(frame.serialize())
                for _ in range(12):
                    frame = ContinuationFrame(1)
                    frame.data = b'\x84'
                    client.writer.write(frame.serialize())
                    await client.writer.drain()
                    await asyncio.sleep(.5)
        except Exception:
            attack[kind + '_closed_or_rejected'] += 1
        finally:
            await client.close()

    async def flood():
        i = 0
        while not stop.is_set():
            task = asyncio.create_task(attack_connection(i))
            attack_tasks.add(task)
            task.add_done_callback(attack_tasks.discard)
            i += 1
            attack['offered'] += 1
            await asyncio.sleep(.0025)

    async def measured(kind, person, client=None):
        start = time.monotonic()
        own = client is None
        client = client or Client(server, client_context, protocol, server.registry.token(person, 0))
        status = None
        error = None
        try:
            if own:
                await client.connect()
            if kind == 'sign_in':
                status, headers, state = await client.request('/auth/start')
                if status == 200:
                    # Local transaction round trip; provider time is NOT measured.
                    await client.close()
                    client = Client(server, client_context, protocol)
                    await client.connect()
                    status, _, _ = await client.request('/auth/callback?state=' + state.decode(), headers.get('set-cookie', ''))
            else:
                status, _, _ = await client.request()
        except Exception as exc:
            error = type(exc).__name__
        finally:
            if own:
                await client.close()
        results.append({'kind': kind, 'person': person, 'status': status, 'error': error,
                        'latency_ms': (time.monotonic()-start)*1000,
                        'at_s': start-began, 'same_source': len(multi_sources) == 1 and multi_sources[0] == '127.0.0.1' or start-began < seconds/2})

    began = time.monotonic()
    attackers = asyncio.create_task(flood())
    while time.monotonic()-began < seconds:
        await asyncio.gather(*(measured('established', person, client) for person, client in sessions),
                             *(measured('reconnect', person) for person in range(25)),
                             *(measured('sign_in', person) for person in range(25)))
        resource_samples.append({'at_s':time.monotonic()-began,'max_rss_bytes':resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,'user_cpu_s':resource.getrusage(resource.RUSAGE_SELF).ru_utime,'system_cpu_s':resource.getrusage(resource.RUSAGE_SELF).ru_stime,'fd_count':len(os.listdir('/dev/fd')),'python_threads':threading.active_count(),'entries':len(server.registry.entries)})
        (root / (protocol.replace('/', '-') + '-partial.json')).write_text(json.dumps({'samples': results, 'metrics': server.registry.metrics, 'attack': attack}))
        await asyncio.sleep(2)
    flood_elapsed_s = time.monotonic()-began
    stop.set()
    await attackers
    await asyncio.gather(*attack_tasks, return_exceptions=True)
    for _, client in sessions:
        await client.close()
    await asyncio.sleep(6)
    accept.cancel()
    await asyncio.gather(accept, return_exceptions=True)
    server.listener.close()
    await asyncio.gather(*server.tasks, return_exceptions=True)
    summary = {}
    for kind in ('established', 'reconnect', 'sign_in'):
        selected = [r for r in results if r['kind'] == kind]
        summary[kind] = {'offered': len(selected), 'success': sum(r['status'] == 200 and r['latency_ms'] <= 5000 for r in selected),
                         'max_ms': max(r['latency_ms'] for r in selected),
                         'per_person': {str(p): {'offered': sum(r['person'] == p for r in selected), 'success': sum(r['person'] == p and r['status'] == 200 and r['latency_ms'] <= 5000 for r in selected)} for p in range(25)}}
    result = {'protocol': protocol, 'duration_s': time.monotonic()-began, 'offered_rate_target_per_s': 400,
              'source_count': [1, len(multi_sources)], 'unavailable_source_bindings': unavailable_sources,
              'continuous_flood_s': flood_elapsed_s,
              'multi_source_qualified': len(multi_sources) > 1,
              'loopback_only': True, 'summary': summary, 'samples': results,
              'metrics': server.registry.metrics, 'attack': attack, 'resource_samples':resource_samples,
              'remaining_entries_after_drain': len(server.registry.entries),
              'limits': ['raw TLS, partial h1/h2 and completed preauth offered concurrently; sustained proof on loopback only',
                         'unavailable source aliases are recorded; same-source flood continues rather than silently losing traffic; unavailable multi-source case FAIL',
                         'synthetic local sessions/callbacks; actual provider sign-ins NOT RUN and required item FAIL',
                         'no A1 search execution, routed NAT/VPN/VDI or desktop-service measurement: FAIL; resource counters include in-process attack clients',
                         'h2 CONTINUATION absolute 5s/16KiB guard implemented; separate hostile deadline assertions still required',
                         'revocation/expiry and fourth-session LRU not qualified: FAIL']}
    (root / (protocol.replace('/', '-') + '.json')).write_text(json.dumps(result, indent=2))
    return result


async def table_trial(context, client_context):
    server = Server(context)
    accept = asyncio.create_task(server.accept())
    loop = asyncio.get_running_loop()
    offered, errors, bound = 0, 0, 0
    error_codes = Counter()
    for i in range(1025):
        raw = socket.socket()
        raw.setblocking(False)
        offered += 1
        try:
            raw.bind((f'127.1.{i//256}.{i%256}', 0))
            bound += 1
            await asyncio.wait_for(loop.sock_connect(raw, ('127.0.0.1', server.port)), 1)
            await asyncio.sleep(.001)
        except (OSError, asyncio.TimeoutError) as exc:
            errors += 1
            error_codes[str(getattr(exc, 'errno', type(exc).__name__))] += 1
        finally:
            raw.close()
    await asyncio.sleep(.1)
    before = len(server.registry.sources)
    unknown = Client(server, client_context, 'http/1.1')
    started = time.monotonic()
    status, error = None, None
    try:
        await unknown.connect('127.99.99.99')
        status, _, _ = await unknown.request('/auth/start')
    except Exception as exc:
        error = type(exc).__name__
    finally:
        await unknown.close()
    unknown_elapsed_ms = (time.monotonic()-started)*1000
    await asyncio.sleep(6)
    accept.cancel()
    await asyncio.gather(accept, return_exceptions=True)
    server.listener.close()
    await asyncio.gather(*server.tasks, return_exceptions=True)
    ipv6 = len({source_key(f'2001:db8:1:2::{i:x}') for i in range(2048)})
    # A separate deterministic model does not turn unavailable kernel source
    # addresses into network evidence. Preserve both denominators explicitly.
    model = Registry()
    now = time.monotonic()
    for i in range(1025):
        model.gate(f'10.1.{i//256}.{i%256}', now+i/100)
        model.entries.clear()
    model_full = len(model.sources)
    model_unknown = model.gate('10.99.99.99', now+11)
    return {'status': 'FAIL', 'offered_kernel_source_addresses': offered, 'connect_errors': errors,
            'bound_kernel_sources': bound, 'error_codes': error_codes,
            'table_size': before, 'table_size_after': len(server.registry.sources),
            'unknown_legitimate_signin': {'offered': 1, 'success': int(status == 200), 'status': status, 'error': error, 'latency_ms':unknown_elapsed_ms},
            'same_ipv6_64_unique_keys': ipv6, 'same_ipv6_64_model_addresses': 2048,
            'separate_model': {'table_size': model_full, 'unknown_rejected': model_unknown is None,
                               'metrics': model.metrics, 'kernel_evidence': False},
            'metrics': server.registry.metrics,
            'kernel_exhaustion_qualified': before == 1024,
            'reason': ('Kernel table exhaustion unqualified: source aliases unavailable; ' if bound < 1025 else '') +
                      'separate model fills 1024 keys and rejects unknown legitimate source; IPv6 same-/64 churn is normalization model only; required case FAIL'}


async def main(args):
    root = args.evidence.resolve()
    repo = Path(__file__).resolve().parents[1]
    if root == repo or repo in root.parents:
        raise ValueError('private evidence required')
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    certificate, key = root / 'cert.pem', root / 'key.pem'
    with (root / 'certificate.log').open('wb') as log:
        subprocess.run([RTK, 'proxy', 'openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
                        '-subj', '/CN=localhost', '-addext', 'subjectAltName=DNS:localhost,IP:127.0.0.1',
                        '-keyout', str(key), '-out', str(certificate)], stdout=log, stderr=subprocess.STDOUT, check=True)
    key.chmod(0o600)
    for protocol in args.protocols:
        await trial(root, protocol, args.seconds, certificate, key)
    server_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    server_context.load_cert_chain(certificate, key)
    server_context.set_alpn_protocols(['http/1.1'])
    client_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    client_context.load_verify_locations(certificate)
    client_context.set_alpn_protocols(['http/1.1'])
    (root / 'table-exhaustion.json').write_text(json.dumps(await table_trial(server_context, client_context), indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--seconds', type=int, default=600)
    parser.add_argument('--protocols', nargs='*', choices=('http/1.1', 'h2'), default=['http/1.1', 'h2'],
                        help='Select trials; an empty list runs only table exhaustion. Does not reuse partial runs.')
    asyncio.run(main(parser.parse_args()))
