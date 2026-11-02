#!/usr/bin/env python3
"""Fail-closed tests for the Amendment 2 ETL layout and capture attribution."""
import importlib.util
import io
import json
from unittest.mock import patch
from pathlib import Path
import struct
import unittest

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
        return [
            # The pre-capture send can precede the in-window socket creation.
            event(18, 1, BufferLength=str(len(a9.PRE_MESSAGE))),
            event(1, 2, UserModePid='42'),
            event(4, 2, Address='127.0.0.1', Port='5555'),
            event(18, 2, BufferLength=str(len(a9.NEGATIVE_MESSAGE))),
        ]

    def check_windows(self, events):
        a9.check_windows(events, 42, [43], 5555,
                         [a9.instant('2026-11-02T12:00:00Z'), a9.instant('2026-11-02T12:00:02Z')], io.StringIO())

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

    def test_macos_precapture_send_and_missing_send_mutation(self):
        packets = [
            f'101.0 (out, proc vectoraiz:42) IP 127.0.0.1.5556 > 127.0.0.1.5555: Flags [P.], length {len(a9.PRE_MESSAGE)}',
            '101.1 (out, proc vectoraiz:42) IP 127.0.0.1.5557 > 127.0.0.1.5555: Flags [S], length 0',
            f'101.2 (out, proc vectoraiz:42) IP 127.0.0.1.5557 > 127.0.0.1.5555: Flags [P.], length {len(a9.NEGATIVE_MESSAGE)}',
        ]
        def check(lines):
            a9.check_macos('\n'.join(lines), 42, [43], 5555, 5556, 5557, [100, 102], io.StringIO())
        check(packets)
        with self.assertRaisesRegex(AssertionError, 'pre-capture send not attributed'):
            check(packets[1:])
        with self.assertRaisesRegex(AssertionError, 'App made a network'):
            check(packets + ['101.3 (out, proc vectoraiz:42) IP 127.0.0.1.5558 > 13.226.220.23.80: Flags [P.], length 12'])


if __name__ == '__main__':
    unittest.main(verbosity=2)
