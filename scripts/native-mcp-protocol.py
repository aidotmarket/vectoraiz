#!/usr/bin/env python3
"""Whole-message validation against the pinned official schemas, in both eras."""
import argparse
import json
from pathlib import Path
import queue
import subprocess
import threading
import jsonschema

ROOT = Path(__file__).resolve().parents[1]


class Client:
    def __init__(self, binary, client, era, evidence, environment=None, client_file=None):
        self.era = era
        self.schema = json.loads((ROOT / f'crates/vz-mcp/schema/{era}.json').read_text())
        self.validator = jsonschema.Draft202012Validator(self.schema)
        self.log = evidence
        if client_file:
            self.log.write(json.dumps({'bridge': str(binary), 'argv': ['--client-file', str(client_file)]}) + '\n')
            self.log.flush()
        self.child = subprocess.Popen([str(binary), *(['--client-file', str(client_file)] if client_file else ['--client', client])], stdin=subprocess.PIPE,
                                      stdout=subprocess.PIPE, stderr=evidence, text=True,
                                      encoding='utf-8', env=environment)
        self.messages = queue.Queue()
        def read():
            for line in self.child.stdout:
                self.messages.put(json.loads(line))  # Non-JSON stdout fails the test.
            self.messages.put(None)
        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()
        self.number = 0

    def validate(self, message, definition):
        # Validate the complete envelope as well as the method-specific payload.
        self.validator.evolve(schema={'$ref': f'#/$defs/{definition}', '$defs': self.schema['$defs']}).validate(message)

    def request(self, method, params=None):
        self.number += 1
        params = {} if params is None else dict(params)
        if self.era == 'modern':
            params['_meta'] = {'io.modelcontextprotocol/protocolVersion': '2026-07-28',
                               'io.modelcontextprotocol/clientCapabilities': {}}
        message = {'jsonrpc': '2.0', 'id': self.number, 'method': method, 'params': params}
        self.validate(message, {'server/discover':'DiscoverRequest', 'tools/list':'ListToolsRequest',
                               'tools/call':'CallToolRequest', 'initialize':'InitializeRequest'}.get(method, 'JSONRPCRequest'))
        return message

    def write(self, message):
        self.log.write(json.dumps({'sent': message}) + '\n'); self.log.flush()
        self.child.stdin.write(json.dumps(message) + '\n'); self.child.stdin.flush()

    def read(self, request, timeout=35):
        response = self.messages.get(timeout=timeout)
        assert response is not None, 'bridge exited before response'
        self.log.write(json.dumps({'received': response}) + '\n'); self.log.flush()
        assert response['id'] == request['id'], response
        if 'error' in response:
            self.validate(response, 'JSONRPCErrorResponse')
        else:
            self.validate(response, 'JSONRPCResultResponse' if self.era == 'modern' else 'JSONRPCResponse')
            definition = {'server/discover':'DiscoverResult', 'tools/list':'ListToolsResult',
                          'tools/call':'CallToolResult', 'initialize':'InitializeResult'}.get(request['method'], 'Result')
            self.validate(response['result'], definition)
            if self.era == 'modern':
                assert response['result']['resultType'] == 'complete'
        return response

    def call(self, method, params=None):
        message = self.request(method, params); self.write(message); return self.read(message)

    def startup(self):
        if self.era == 'modern':
            self.call('server/discover')
            bad = self.request('ping'); bad['params']['_meta']['io.modelcontextprotocol/protocolVersion'] = 'unknown'
            self.write(bad); response = self.read(bad); assert response['error']['code'] == -32022
            self.validate(response, 'UnsupportedProtocolVersionError')
            for field in ['io.modelcontextprotocol/clientCapabilities', 'io.modelcontextprotocol/protocolVersion']:
                bad = self.request('ping'); del bad['params']['_meta'][field]
                self.write(bad); assert self.read(bad)['error']['code'] == -32602
        else:
            self.call('initialize', {'protocolVersion':'2025-11-25', 'capabilities':{},
                                     'clientInfo':{'name':'M3 validation', 'version':'1'}})
            notice = {'jsonrpc':'2.0', 'method':'notifications/initialized'}
            self.validate(notice, 'InitializedNotification'); self.write(notice)
        self.call('ping')
        listed = self.call('tools/list')['result']['tools']
        assert {t['name'] for t in listed} == {'list_collections', 'search', 'get_document', 'get_chunk'}
        for tool in listed:
            assert tool['annotations']['readOnlyHint'] is True
            assert tool['annotations']['openWorldHint'] is False
            jsonschema.Draft202012Validator.check_schema(tool['inputSchema'])
            jsonschema.Draft202012Validator.check_schema(tool['outputSchema'])
        self.tools = {tool['name']: tool for tool in listed}

    def tools_run(self, allowed, denied='vectorAIz has not given this app access.'):
        hits = self.tool('search', {'query':'thirty days'}, allowed, denied)
        hit = hits['hits'][0] if allowed else {'doc_id':'unknown', 'chunk_id':'unknown'}
        self.tool('list_collections', None, allowed, denied)
        self.tool('get_document', {'doc_id':hit['doc_id']}, allowed, denied)
        self.tool('get_chunk', {'chunk_id':hit['chunk_id']}, allowed, denied)

    def tool(self, name, arguments, allowed, denied='vectorAIz has not given this app access.'):
        params = {'name':name}
        if arguments is not None: params['arguments'] = arguments
        response = self.call('tools/call', params)['result']
        if allowed:
            assert not response.get('isError'), response
            obj = response['structuredContent']; assert isinstance(obj, dict)
            jsonschema.validate(obj, self.tools[name]['outputSchema'])
            assert json.loads(response['content'][0]['text']) == obj
            return obj
        assert response['isError'] is True
        assert response['content'][0]['text'] == denied

    def close(self):
        self.child.stdin.close()
        assert self.child.wait(timeout=10) == 0
        self.reader.join(timeout=5)
        assert self.messages.get(timeout=5) is None, 'unexpected extra protocol response'


def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--bridge', type=Path, required=True)
    args = parser.parse_args()
    for era in ['modern', 'legacy']:
        with Path(f'mcp-protocol-{era}.jsonl').open('w', encoding='utf-8') as evidence:
            client = Client(args.bridge.resolve(), 'malformed', era, evidence)
            try:
                # A4.7 forwards invalid IDs as empty; with no app listening, expect NOT_RUNNING.
                client.startup(); client.tools_run(False, denied='Open vectorAIz to search your library.')
            finally:
                client.close()
        print(f'{era}: whole-message schema validation PASS', flush=True)


if __name__ == '__main__':
    main()
