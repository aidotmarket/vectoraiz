#!/usr/bin/env python3
"""Prepare private SciFact pins and 60 CC0 office drafts, then score runfiles.

Draft expected sources are author intent, never human judgments. Two blinded
human judges, adjudication and frozen office qrels remain required.
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import zipfile


def digest(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def prepare(root):
    from docx import Document
    from openpyxl import Workbook
    from PIL import Image, ImageDraw, ImageFont
    from reportlab.pdfgen.canvas import Canvas
    from reportlab.pdfbase import pdfmetrics
    from reportlab.pdfbase.cidfonts import UnicodeCIDFont
    from reportlab.lib.utils import ImageReader
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    with zipfile.ZipFile(root / 'scifact.zip') as archive:
        for item in archive.infolist():
            destination = (root / item.filename).resolve()
            if root.resolve() not in destination.parents:
                raise ValueError('unsafe archive path')
        archive.extractall(root)
    fixtures = root / 'office-drafts'
    fixtures.mkdir(exist_ok=True)
    (fixtures / 'LICENSE-CC0.txt').write_text('SPDX-License-Identifier: CC0-1.0\n'
        'These 60 original harness documents and draft questions are dedicated to the public domain under CC0 1.0.\n'
        'https://creativecommons.org/publicdomain/zero/1.0/legalcode\n')
    templates = {
        'en': ('Office procurement memorandum', 'The supplier requires approval before the order is placed.',
               'Approved limit: {amount} euros. Delivery date: 18 December 2026.',
               'For project {code}, how much may purchasing spend and when is delivery due?'),
        'de': ('Vermerk zur Beschaffung', 'Vor der Bestellung muss der Lieferant genehmigt werden.',
               'Genehmigte Obergrenze: {amount} Euro. Liefertermin: 18. Dezember 2026.',
               'Wie hoch ist das Einkaufsbudget für Projekt {code} und wann wird geliefert?'),
        'fr': ('Note sur les achats', 'Le fournisseur doit être approuvé avant la commande.',
               'Plafond approuvé : {amount} euros. Livraison : le 18 décembre 2026.',
               'Quel budget est autorisé pour le projet {code} et quelle est la date de livraison ?'),
        'es': ('Nota de compras', 'El proveedor debe aprobarse antes de realizar el pedido.',
               'Límite aprobado: {amount} euros. Entrega: 18 de diciembre de 2026.',
               '¿Cuánto se puede gastar en el proyecto {code} y cuándo debe llegar el pedido?'),
        'ja': ('調達に関する業務連絡', '発注する前に仕入先の承認が必要です。',
               '承認済み予算上限：{amount}ユーロ。納品日：2026年12月18日。',
               'プロジェクト{code}の購入予算と納品予定日は何ですか。'),
    }
    pdfmetrics.registerFont(UnicodeCIDFont('HeiseiMin-W3'))
    rows = []
    for format_number, extension in enumerate(('pdf', 'docx', 'xlsx', 'txt', 'md', 'html')):
        for i in range(10):
            language = list(templates)[i % 5]
            heading, context, answer, query = templates[language]
            code = f'ORBIT-{format_number:02}-{i:02}'
            amount = 12000 + format_number*1000 + i*137
            answer = answer.format(amount=amount)
            paragraphs = [heading + ' ' + code, context, context*45,
                          'Table / Tabelle / Tableau / 表: ' + code, answer,
                          'Duplicate policy: approval is required before purchase.']
            text = '\n\n'.join(paragraphs)
            path = fixtures / f'{format_number:02}-{i:02}-{language}.{extension}'
            scanned = extension == 'pdf' and i in (0, 4)
            if extension == 'txt':
                path.write_text(text)
            elif extension == 'md':
                path.write_text('# ' + paragraphs[0] + '\n\n' + '\n\n'.join(paragraphs[1:]))
            elif extension == 'html':
                import html
                path.write_text('<!doctype html><meta charset="utf-8"><html lang="' + language + '"><body>' +
                                ''.join('<p>' + html.escape(p) + '</p>' for p in paragraphs) +
                                '<table><tr><th>Project</th><th>Budget</th></tr><tr><td>' + code +
                                '</td><td>' + str(amount) + '</td></tr></table></body></html>')
            elif extension == 'docx':
                document = Document()
                for p in paragraphs[:3]:
                    document.add_paragraph(p)
                document.add_page_break()
                for p in paragraphs[3:]:
                    document.add_paragraph(p)
                table = document.add_table(rows=2, cols=2)
                table.cell(0, 0).text = code
                table.cell(1, 1).text = str(amount)
                document.save(path)
            elif extension == 'xlsx':
                workbook = Workbook()
                sheet = workbook.active
                sheet.title = 'Context'
                for p in paragraphs[:3]:
                    sheet.append([p])
                sheet = workbook.create_sheet('Approval')
                sheet.append(['Project', 'Budget', 'Delivery'])
                sheet.append([code, amount, '2026-12-18'])
                sheet.append([answer])
                workbook.save(path)
            else:
                canvas = Canvas(str(path))
                if scanned:
                    image = Image.new('RGB', (1200, 1600), 'white')
                    draw = ImageDraw.Draw(image)
                    font = ImageFont.truetype('/System/Library/Fonts/PingFang.ttc', 24)
                    for line, p in enumerate((paragraphs[0], context, answer)):
                        draw.text((40, 80 + line*100), p, fill='black', font=font)
                    canvas.drawImage(ImageReader(image), 0, 0, width=595, height=795)
                    canvas.showPage()
                else:
                    for page in (paragraphs[:2], paragraphs[3:]):
                        canvas.setFont('HeiseiMin-W3' if language == 'ja' else 'Helvetica', 11)
                        for line, p in enumerate(page):
                            canvas.drawString(30, 760-line*50, p)
                        canvas.showPage()
                canvas.save()
            rows.append({'id': code, 'path': str(path), 'format': extension, 'language': language,
                         'sha256': digest(path), 'query': query.format(code=code),
                         'draft_support': answer, 'scanned': scanned,
                         'judgment_status': 'UNJUDGED', 'support_location': 'Approval sheet or final content page/passage'})
    (root / 'office-drafts.json').write_text(json.dumps(rows, ensure_ascii=False, indent=2))
    pinned = [root / 'SciFact-LICENSE.md', root / 'BEIR-README.md', root / 'scifact.zip',
              root / 'scifact/corpus.jsonl', root / 'scifact/queries.jsonl', root / 'scifact/qrels/test.tsv', root / 'CC-BY-4.0.txt', root / 'CC-BY-SA-4.0.txt', root / 'ODC-By-1.0.html', root / 'CC0-1.0.txt',
              fixtures / 'LICENSE-CC0.txt', root / 'office-drafts.json']
    (root / 'fixture-pins.json').write_text(json.dumps({
        'files': {str(p.relative_to(root)): {'sha256': digest(p), 'bytes': p.stat().st_size} for p in pinned},
        'attribution': 'SciFact: Wadden et al., Fact or Fiction: Verifying Scientific Claims, EMNLP 2020; BEIR conversion: Thakur et al., BEIR, 2021.',
        'licenses': {'claims_annotations': 'CC-BY-4.0', 'abstract_database': 'ODC-By-1.0',
                     'BEIR_conversion': 'CC-BY-SA-4.0', 'office_drafts': 'CC0-1.0'},
        'office_counts': {'total': 60, 'queries': 60, 'per_format': 10, 'per_language': 12, 'scans': 2},
        'missing': ['two independent human judges for all 60 queries', 'blinded pooled exact/ANN/FTS grading 0/1/2',
                    'disagreement adjudication', 'frozen judged office qrels', 'supporting passages verified after extraction',
                    'hostile, image and accessibility content comparisons'],
    }, indent=2))


def score(root):
    qrels = {}
    with (root / 'scifact/qrels/test.tsv').open() as file:
        for row in csv.DictReader(file, delimiter='\t'):
            qrels.setdefault(row['query-id'], {})['scifact-' + row['corpus-id']] = int(row['score'])
    runs = json.loads((root / 'scifact-run.json').read_text())
    scored = {}
    for name, rows in runs['runs'].items():
        if len(rows) != len(qrels) or {r['id'] for r in rows} != set(qrels):
            raise ValueError('incomplete/duplicate human test-query run; cannot publish aggregate scores')
        values = []
        for row in rows:
            rel = qrels[row['id']]
            documents = list(dict.fromkeys(row['documents']))
            metrics = {}
            for k in (8, 10):
                ideal = sorted(rel.values(), reverse=True)[:k]
                dcg = sum((2**rel.get(doc, 0)-1)/math.log2(i+2) for i, doc in enumerate(documents[:k]))
                idcg = sum((2**grade-1)/math.log2(i+2) for i, grade in enumerate(ideal))
                metrics[f'recall@{k}'] = sum(rel.get(doc, 0) > 0 for doc in documents[:k])/sum(v > 0 for v in rel.values())
                metrics[f'nDCG@{k}'] = dcg/idcg if idcg else None
            values.append({'id': row['id'], **metrics})
        scored[name] = {'per_query': values, 'mean': {key: sum(v[key] for v in values)/len(values) for key in values[0] if key != 'id'}}
    baseline = scored['exact']['mean']
    for name, result in scored.items():
        result['ratio_to_exact'] = {key: value/baseline[key] if baseline[key] else None for key, value in result['mean'].items()}
        reference = {row['id']: row for row in scored['exact']['per_query']}
        result['per_query_delta_from_exact'] = [
            {'id': row['id'], **{key: value-reference[row['id']][key] for key, value in row.items() if key != 'id'}}
            for row in result['per_query']]
        result['floor_0_98'] = {key: ('PASS' if ratio >= .98 else 'FAIL') if ratio is not None else 'FAIL: baseline zero; human qualification required'
                               for key, ratio in result['ratio_to_exact'].items()}
    office_runs = json.loads((root / 'office-run.json').read_text())
    diagnostics = {}
    for name in sorted({row['path'] for row in office_runs}):
        selected = [r for r in office_runs if r['path'] == name]
        drafts = json.loads((root / 'office-drafts.json').read_text())
        if len(selected) != len(drafts) or {r['id'] for r in selected} != {r['id'] for r in drafts}:
            raise ValueError('incomplete/duplicate office diagnostic run')
        diagnostics[name] = {
            'offered': len(selected), 'author_intended_doc_in_chunk_top10': sum(r['id'] in r['documents'] for r in selected),
            'by_format': {fmt: {'offered': sum(r['format'] == fmt for r in selected),
                               'author_intended_doc_in_chunk_top10': sum(r['format'] == fmt and r['id'] in r['documents'] for r in selected)}
                          for fmt in ('pdf', 'docx', 'xlsx', 'txt', 'md', 'html')},
            'qualification': 'UNJUDGED author-intent diagnostic; not recall/nDCG or A1 correctness',
        }
    (root / 'scores.json').write_text(json.dumps({'SciFact': scored, 'office': {'status': 'FAIL', 'reason': '60 unjudged drafts; no invented human qrels'},
                                                'office_diagnostics': diagnostics,
                                                'scope': runs['scope']}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('prepare', 'score'))
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    if Path(__file__).resolve().parents[1] in args.evidence.resolve().parents:
        raise ValueError('private evidence only')
    (prepare if args.mode == 'prepare' else score)(args.evidence.resolve())
