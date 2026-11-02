#!/usr/bin/env python3
"""Fail-closed tests for the Amendment 2 ETL layout and capture attribution."""
import importlib.util
import io
import json
from unittest.mock import patch
from pathlib import Path
import struct
import unittest
import tempfile

spec = importlib.util.spec_from_file_location('a9', Path(__file__).with_name('native-mcp-a9.py'))
a9 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a9)


class CaptureTests(unittest.TestCase):
    def test_etl_header_and_truncation(self):
        header = bytearray(0x186)
        header[0x180:] = 'a9\0'.encode('utf-16le')
        self.assertEqual(a9.etl_losses(header), (0, 0))
        struct.pack_into('<I', header, 0x68 + 48, 7)
        struct.pack_into('<I', header, 0x68 + 276, 9)
        self.assertEqual(a9.etl_losses(header), (7, 9))
        for length in [0, 0x68, 0x17f, 0x180, 0x185]:
            with self.subTest(length=length), self.assertRaises(ValueError):
                a9.etl_losses(header[:length])
        header[0x180] = ord('b')
        with self.assertRaises(ValueError):
            a9.etl_losses(header)

    def test_enumeration_classifies_instead_of_rejecting_all_tcp(self):
        addresses = {'ai.market': ['192.0.2.1']}
        established = dict(State=5, RemoteAddress='13.226.220.23', RemotePort=80)
        bound = dict(State=100, RemoteAddress='0.0.0.0', RemotePort=0)
        with patch.object(a9.platform, 'system', return_value='Windows'):
            for tcp, udp, fails in [
                ([bound, established], [], False),
                ([dict(established, State=2)], [], True),
                ([dict(established, RemoteAddress='192.0.2.1')], [], True),
                ([], [dict(LocalPort=123)], True),
            ]:
                with self.subTest(tcp=tcp, udp=udp), patch.object(a9, 'run', return_value=json.dumps(dict(tcp=tcp, udp=udp))):
                    if fails:
                        with self.assertRaises(AssertionError):
                            a9.enumerate_sockets([42], addresses, io.StringIO())
                    else:
                        a9.enumerate_sockets([42], addresses, io.StringIO())
        with patch.object(a9.platform, 'system', return_value='Darwin'):
            for entry, fails in [
                ('TCP 127.0.0.1:111->13.226.220.23:80 (ESTABLISHED)', False),
                ('TCP 127.0.0.1:111->192.0.2.1:443 (ESTABLISHED)', True),
                ('TCP *:111 (LISTEN)', True), ('UDP *:111', True),
            ]:
                output = 'COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nvectoraiz 42 user 1u IPv4 0 0 ' + entry
                with self.subTest(entry=entry), patch.object(a9, 'run', return_value=output):
                    if fails:
                        with self.assertRaises(AssertionError):
                            a9.enumerate_sockets([42], addresses, io.StringIO())
                    else:
                        a9.enumerate_sockets([42], addresses, io.StringIO())

    def windows_fixture(self):
        def event(event_id, endpoint, **fields):
            return event_id, dict(Process='0x1234', Endpoint=str(endpoint),
                                 TimeCreated='2026-11-02T12:00:01Z', **fields)
        events = [
            # The pre-capture send can precede the in-window socket creation.
            event(18, 1, BufferLength=str(len(a9.PRE_MESSAGE))),
            event(1, 2, UserModePid='42'),
            event(4, 2, Address='127.0.0.1', Port='5555'),
            event(18, 2, BufferLength=str(len(a9.NEGATIVE_MESSAGE))),
        ]
        for _, fields in events:
            fields['_version'] = '0'
            fields['_payload'] = list(fields)
        return events

    def fixture_schema(self):
        templates = {1: {'Process', 'Endpoint', 'UserModePid'},
                     4: {'Process', 'Endpoint', 'Address', 'Port'},
                     18: {'Process', 'Endpoint', 'BufferLength'}}
        return ({(1, 0): 'create', (4, 0): 'connect', (18, 0): 'send'},
                {(key, 0): value for key, value in templates.items()})

    def check_windows(self, events):
        a9.check_windows(events, 42, [43], 5555,
                         [a9.instant('2026-11-02T12:00:00Z'), a9.instant('2026-11-02T12:00:02Z')], io.StringIO(), self.fixture_schema())

    def test_windows_precapture_attribution_and_missing_send_mutation(self):
        events = self.windows_fixture()
        self.check_windows(events)
        with self.assertRaisesRegex(AssertionError, 'pre-capture send not attributed'):
            self.check_windows(events[1:])
        with self.assertRaisesRegex(AssertionError, 'App/bridge sent'):
            self.check_windows(events + [(18, dict(events[0][1], Endpoint='3', BufferLength='99'))])
        with self.assertRaisesRegex(AssertionError, 'extra app connection'):
            self.check_windows(events + [(4, dict(events[2][1], Endpoint='3', Port='80', Address='13.226.220.23'))])
        events[0][1]['TimeCreated'] = '2026-11-02T11:59:59Z'
        with self.assertRaisesRegex(AssertionError, 'pre-capture send not attributed'):
            self.check_windows(events)

    def test_windows_xml_uses_payload_pid_and_rejects_decode_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'trace.xml'
            trace = f'''<Events><Event><System>
                <Provider Guid="{{{a9.AFD_GUID}}}"/><EventID>7300</EventID><Version>0</Version>
                <Execution ProcessID="999"/><TimeCreated SystemTime="2026-11-02T12:00:01Z"/>
                </System><EventData><Data Name="Process">0x1234</Data>
                <Data Name="ProcessId">42</Data><Data Name="Endpoint">2</Data></EventData>
                </Event></Events>'''
            path.write_text(trace)
            events = a9.windows_events(path)
            self.assertEqual(events[0][1]['ProcessId'], '42')
            self.assertNotIn('ProcessID', events[0][1])
            path.write_text(trace.replace('<EventData>', '<ProcessingErrorData/><EventData>'))
            with self.assertRaisesRegex(AssertionError, 'undecodable AFD'):
                a9.windows_events(path)

    def test_windows_runner_manifest_mapping_and_mutation(self):
        # IDs are deliberately arbitrary: names/templates determine roles.
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            manifest = directory / 'manifest.xml'; schema_path = directory / 'schema.json'
            schema = dict(provider='Microsoft-Windows-Winsock-AFD', guid=a9.AFD_GUID, events=[])
            for event_id, task, fields in [
                (7300, 'AfdCreate', ['Process', 'Endpoint', 'ProcessId']),
                (7318, 'AfdConnectWithAddress', ['Process', 'Endpoint', 'Address']),
                (7303, 'AfdSend', ['Process', 'Endpoint', 'BufferLength']),
            ]:
                schema['events'].append(dict(id=event_id, version=0, task=task,
                    template='<template>' + ''.join(f'<data name="{f}"/>' for f in fields) + '</template>'))
            manifest.write_text('<providerMetadata><events>' + ''.join(
                f'<event value="{e["id"]}" version="0"/>' for e in schema['events']) + '</events></providerMetadata>')
            schema_path.write_text(json.dumps(schema))
            mapping = a9.windows_schema(schema_path, manifest)
            events = self.windows_fixture()
            renumber = {1: 7300, 4: 7318, 18: 7303}
            events = [(renumber[event_id], dict(fields)) for event_id, fields in events]
            events[1][1]['ProcessId'] = events[1][1].pop('UserModePid')
            events[2][1]['Address'] = '127.0.0.1:5555'
            events[2][1].pop('Port')
            for _, fields in events: fields['_payload'] = list(fields)
            window = [a9.instant('2026-11-02T12:00:00Z'), a9.instant('2026-11-02T12:00:02Z')]
            evidence = io.StringIO()
            endpoint = a9.check_windows(events, 42, [43], 5555, window, evidence, mapping)
            a9.windows_mutation(events, 42, [43], 5555, window, evidence, mapping, endpoint)
            self.assertIn('Mutation PASS', evidence.getvalue())
            with self.assertRaisesRegex(AssertionError, 'zero recognised'):
                a9.check_windows(events[:1] + events[2:], 42, [43], 5555, window, evidence, mapping)
            with self.assertRaisesRegex(AssertionError, 'unknown AFD event/version'):
                a9.check_windows(events + [(9999, dict(events[0][1]))], 42, [43], 5555, window, evidence, mapping)
            with self.assertRaisesRegex(AssertionError, 'non-control socket'):
                a9.check_windows(events + [(7300, dict(events[1][1], Endpoint='99'))], 42, [43], 5555, window, evidence, mapping)
            with self.assertRaisesRegex(AssertionError, 'no app-owned'):
                changed = [(i, dict(f, ProcessId='43')) if i == 7300 else (i, f) for i, f in events]
                a9.check_windows(changed, 42, [43], 5555, window, evidence, mapping)
            with self.assertRaisesRegex(AssertionError, 'unknown AFD creation PID template'):
                schema['events'][0]['template'] = '<template><data name="Process"/><data name="Endpoint"/></template>'
                schema_path.write_text(json.dumps(schema))
                a9.windows_schema(schema_path, manifest)
            with self.assertRaisesRegex(AssertionError, 'metadata/manifest mismatch'):
                schema['events'][0]['id'] = 8888
                schema_path.write_text(json.dumps(schema))
                a9.windows_schema(schema_path, manifest)

    def test_macos_precapture_send_and_missing_send_mutation(self):
        packets = [
            f'101.0 (proc vectoraiz:42, out) IP 127.0.0.1.5556 > 127.0.0.1.5555: Flags [P.], length {len(a9.PRE_MESSAGE)}',
            '101.1 (proc vectoraiz:42, out) IP 127.0.0.1.5557 > 127.0.0.1.5555: Flags [S], length 0',
            f'101.2 (proc vectoraiz:42, out) IP 127.0.0.1.5557 > 127.0.0.1.5555: Flags [P.], length {len(a9.NEGATIVE_MESSAGE)}',
        ]
        def check(lines):
            a9.check_macos('\n'.join(lines), 42, [43], 5555, 5556, 5557, [100, 102], io.StringIO())
        check(packets)
        check(packets + ['101.3 (proc vectoraiz:42, in) IP 13.226.220.23.80 > 127.0.0.1.5558: Flags [P.], length 12'])
        with self.assertRaisesRegex(AssertionError, 'App made a network'):
            check(packets + ['101.3 (proc vectoraiz:42, out) IP 127.0.0.1.5558 > 13.226.220.23.80: Flags [P.], length 12 in,'])
        with self.assertRaisesRegex(AssertionError, 'unrecognised pktap'):
            check([line.replace(', out)', ', unknown)') for line in packets])
        with self.assertRaisesRegex(AssertionError, 'pre-capture send not attributed'):
            check(packets[1:])
        with self.assertRaisesRegex(AssertionError, 'App made a network'):
            check(packets + ['101.3 (proc vectoraiz:42, out) IP 127.0.0.1.5558 > 13.226.220.23.80: Flags [P.], length 12'])


if __name__ == '__main__':
    unittest.main(verbosity=2)
