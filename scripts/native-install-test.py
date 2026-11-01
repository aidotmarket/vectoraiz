#!/usr/bin/env python3
"""Fresh hosted-runner stand-in: install downloaded artifact, launch UI, uninstall."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import sqlite3
import subprocess
import tempfile
import time


def step(message):
    print(f'[{time.strftime("%H:%M:%S")}] {message}', flush=True)


def run(*args, timeout=90):
    step(f'Run (timeout {timeout}s): ' + subprocess.list2cmdline(list(map(str, args))))
    try:
        result = subprocess.run(list(map(str, args)), timeout=timeout, check=False)
    except subprocess.TimeoutExpired:
        step('TIMEOUT: collecting process diagnostics')
        if platform.system() == 'Windows':
            subprocess.run(['powershell.exe', '-NoProfile', '-Command',
                            'Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,CommandLine | Format-List'],
                           timeout=20, check=False)
        raise
    # Windows Installer may return success with reboot required/scheduled.
    assert result.returncode in (0, 1641, 3010), f'Command failed: {result.returncode}'
    step(f'Completed: exit {result.returncode}')


step('Starting fresh hosted-runner install stand-in')
artifacts = Path('artifacts').resolve()
data = Path.home() / 'vectorAIz'
assert not data.exists(), 'Install test requires a fresh user data folder'
windows = platform.system() == 'Windows'
report = Path('installed-self-test.json').resolve()
env = dict(os.environ, VZ_INSTALLED_SELF_TEST_REPORT=str(report))
if windows:
    installer, = artifacts.rglob('*.msi')
    installed = Path(os.environ['ProgramFiles']) / 'vectorAIz'
    # Use the native system binary, with an explicit log created before launching.
    # Do not resolve msiexec through runner PATH or use shell elevation/UI prompts.
    msiexec = Path(os.environ['SystemRoot']) / 'System32/msiexec.exe'
    log = Path('msi-install.log').resolve()
    log.touch()
    # Use the MSI's per-machine default destination. Python quotes a spaced
    # INSTALLDIR=... argument as a whole; msiexec requires quoting its value
    # and can otherwise open its usage dialog even when /qn was supplied.
    run(msiexec, '/i', installer, '/qn', '/norestart', '/l*v', log,
        'ALLUSERS=1', timeout=240)
    binary = installed / 'vectoraiz.exe'
else:
    installer, = artifacts.rglob('*.dmg')
    mount = Path(tempfile.mkdtemp(prefix='vz-dmg-'))
    run('hdiutil', 'attach', installer, '-nobrowse', '-mountpoint', mount)
    installed = Path('/Applications/vectorAIz.app')
    try:
        run('ditto', mount / 'vectorAIz.app', installed)
    finally:
        run('hdiutil', 'detach', mount)
        mount.rmdir()
    run('codesign', '--verify', '--deep', '--strict', installed)
    binary = installed / 'Contents/MacOS/vectoraiz'
assert binary.is_file(), f'Installed executable missing: {binary}'
step(f'Installed executable: {binary}')
try:
    # No checkout/assets in this working directory: exercise installed paths.
    with tempfile.TemporaryDirectory(prefix='vz-first-run-') as cwd:
        # A per-machine app cannot write WebView2's default profile next to its
        # executable in Program Files. Match Tauri's user-scoped production profile.
        env['WEBVIEW2_USER_DATA_FOLDER'] = str(data / 'webview2')
        step('Launch installed WebView self-test (120s report deadline)')
        ui_log = Path('installed-ui.log').resolve().open('w')
        child = subprocess.Popen([str(binary), '--smoke-test', '--cpu'], cwd=cwd, env=env,
                                 stdout=ui_log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 120
            while not report.exists() and time.monotonic() < deadline and child.poll() is None:
                time.sleep(0.5)
            assert report.is_file(), 'Installed WebView self-test did not return a report'
            step('Self-test report received')
            result = json.loads(report.read_text())
            assert result['status'] == 'OK', result
            assert child.wait(timeout=20) == 0
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=10)
            ui_log.close()
    # Acquisition and pipeline run while the installed binary still exists.
    # Budgets: install 240s + self-test 140s + acquisition 900s + pipeline 300s
    # + startup 30s + single-instance 180s + uninstall 120s < CI step 35 minutes.
    with tempfile.TemporaryDirectory(prefix='vz-acquisition-run-') as cwd:
        models = data / 'models'
        assert not models.exists(), 'Acquisition must start with an empty model directory'
        models.mkdir()
        loaded_report = Path('installed-loaded-models.json').resolve()
        pipeline_env = dict(env, VZ_ACQUISITION_SMOKE='1',
                            VZ_SMOKE_MODELS=str(Path('spike/fixtures/models.json').resolve()),
                            VZ_PIPELINE_FIXTURE=str(Path('spike/fixtures/documents').resolve()),
                            VZ_LOADED_MODELS_REPORT=str(loaded_report))
        with Path('installed-pipeline.log').open('w', encoding='utf-8') as log:
            step('Launch installed acquisition (900s) then pipeline (300s); process budget 1230s')
            child = subprocess.Popen([str(binary), '--pipeline-smoke-test', '--cpu'], cwd=cwd,
                                     env=pipeline_env, stdout=log, stderr=subprocess.STDOUT)
            try:
                assert child.wait(timeout=1230) == 0, 'Installed acquisition/pipeline failed'
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=10)
        assert 'VECTORAIZ_PIPELINE_SMOKE_TEST_OK' in Path('installed-pipeline.log').read_text(encoding='utf-8'), 'No pipeline completion marker'
        pipeline_output = Path('installed-pipeline.log').read_text(encoding='utf-8')
        assert 'search-only: empty-cache Ask returned no_answer before indexing' in pipeline_output
        offline_output = pipeline_output.split('offline: start', 1)[1].split('offline: complete', 1)[0]
        assert 'VECTORAIZ_MODEL_TRANSFER:' not in offline_output, 'Offline pipeline emitted acquisition progress'
        assert not list(models.glob('*.partial')), 'Offline pipeline left a partial file'
        loaded = {Path(item['path']).resolve(): item for item in json.loads(loaded_report.read_text())}
        model_hashes = {}
        for pin in json.loads(Path('spike/fixtures/models.json').read_text()).values():
            path = (models / pin['file']).resolve()
            assert path.stat().st_size == pin['size'], f'Wrong acquired model size: {path}'
            with path.open('rb') as stream:
                digest = hashlib.file_digest(stream, 'sha256').hexdigest()
            assert digest == pin['sha256'], f'Wrong acquired model hash: {path}'
            assert loaded[path]['size'] == pin['size'] and loaded[path]['sha256'] == digest, 'Engine loaded different model files'
            model_hashes[path] = digest
        step('Fixture embedding model and added answer model are the exact files passed to the engine')
        spec = importlib.util.spec_from_file_location('single_instance', Path('scripts/native-single-instance-test.py').resolve())
        single_instance = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(single_instance)
        focus_results = single_instance.test(binary, cwd=cwd, env=env,
                                             result_path='installed-single-instance.json')
        Path('installed-single-instance.json').write_text(json.dumps(focus_results, indent=2))
    # Only the installed job fetches the second pin. Keep CLI fixtures unchanged.
    fixture_spec = importlib.util.spec_from_file_location('fetch_assets', Path('scripts/fetch-assets.py').resolve())
    fetcher = importlib.util.module_from_spec(fixture_spec)
    fixture_spec.loader.exec_module(fetcher)
    tiers_manifest = json.loads(Path('spike/fixtures/models.tiers.json').read_text())
    answers = [tier['pin'] for tier in tiers_manifest['answer_tiers']]
    second = answers[1]
    with tempfile.TemporaryDirectory(prefix='vz-migration-fixtures-') as fixture_dir:
        second_file = Path(fixture_dir) / second['file']
        fetcher.fetch(second['url'], second['sha256'], second_file, second['size'])
        cli = binary.parent / ('vectoraiz-cli.exe' if windows else 'vectoraiz-cli')
        assert cli.is_file(), 'Installed same-commit CLI missing'
        for scenario in ['selected-with-records', 'single-no-selected', 'selected-without-records']:
            with tempfile.TemporaryDirectory(prefix='vz-legacy-' + scenario + '-') as seed_dir:
                seed = Path(seed_dir)
                seed_models = seed / 'models'
                seed_models.mkdir()
                shutil.copytree(models / 'pdfium', seed_models / 'pdfium')
                pins = [tiers_manifest['embedding'], answers[0]]
                if scenario != 'single-no-selected':
                    pins.append(answers[1])
                for pin in pins:
                    source = second_file if pin['sha256'] == second['sha256'] else models / pin['file']
                    shutil.copy2(source, seed_models / pin['file'])
                # Same recipe and document ids/generations as a legacy 0.0.13 index.
                legacy = seed / 'phase0-index'
                resolved = seed_models / 'models.selected.json'
                resolved.write_text(json.dumps({'embedding':tiers_manifest['embedding'],'chat':answers[0]}))
                with Path('installed-migration-' + scenario + '-seed.log').open('w', encoding='utf-8') as log:
                    seeded = subprocess.run([str(cli),'--cpu','--state',str(legacy),'--assets',str(seed_models),'--models',str(resolved),'index',str(Path('spike/fixtures/documents').resolve())],stdout=log,stderr=subprocess.STDOUT,timeout=120,check=False)
                    assert seeded.returncode == 0, 'Legacy fixture index failed'
                    # Save a verify-once answer record, as 0.0.13 would have done.
                    checked = subprocess.run([str(cli),'--cpu','--state',str(legacy),'--assets',str(seed_models),'--models',str(resolved),'ask','What is the orbital archive retention policy?'],stdout=log,stderr=subprocess.STDOUT,timeout=120,check=False)
                    assert checked.returncode == 0, 'Legacy fixture Ask failed'
                if scenario == 'single-no-selected':
                    resolved.unlink()
                if scenario == 'selected-without-records':
                    (seed_models / 'models.verified.json').unlink()
                def logical_index():
                    with sqlite3.connect(legacy / 'state.sqlite') as database:
                        documents = database.execute('SELECT doc_id,generation FROM documents ORDER BY doc_id').fetchall()
                    return (legacy.joinpath('recipe.json').read_bytes(), documents)
                before_index = logical_index()
                stamps = {pin['file']:(seed_models / pin['file']).stat() for pin in pins}
                migration_env = dict(env, VZ_SMOKE_DATA_ROOT=str(seed),
                                     WEBVIEW2_USER_DATA_FOLDER=str(seed / 'webview2'),
                                     VZ_SMOKE_MODELS=str(Path('spike/fixtures/models.tiers.json').resolve()),
                                     VZ_PIPELINE_FIXTURE=str(Path('spike/fixtures/documents').resolve()))
                for launch in range(2):
                    migration_env['VZ_MIGRATION_SMOKE'] = 'check'
                    migration_log = Path(f'installed-migration-{scenario}-{launch}.log')
                    with migration_log.open('w',encoding='utf-8') as log:
                        child = subprocess.Popen([str(binary),'--pipeline-smoke-test','--cpu'],cwd=fixture_dir,env=migration_env,stdout=log,stderr=subprocess.STDOUT)
                        try:
                            assert child.wait(timeout=150) == 0, 'Installed migration failed'
                        finally:
                            if child.poll() is None:
                                child.kill(); child.wait(timeout=10)
                    output = migration_log.read_text(encoding='utf-8')
                    assert 'migration: searched before indexing with adopted tier 0' in output
                    assert 'VECTORAIZ_MODEL_TRANSFER:' not in output, 'Migration acquired a model'
                    assert not list(seed_models.glob('*.partial')), 'Migration left a partial'
                    assert logical_index() == before_index, 'Migration changed index recipe or documents/generations'
                    for filename,stamp in stamps.items():
                        current = (seed_models / filename).stat()
                        assert (current.st_size,current.st_mtime_ns,current.st_ino) == (stamp.st_size,stamp.st_mtime_ns,stamp.st_ino), 'Migration changed model stamps'
                    selection = json.loads((seed_models / 'settings.json').read_text())
                    assert selection['answer'] == {'local_pin_sha256':answers[0]['sha256'],'selection_initialised':True}
                if scenario == 'selected-without-records':
                    # Finish the installed target's cold verification outside the held
                    # Search's 10s deadline. The smoke hold makes its checking/transfer
                    # phase observable before the short record check and activation.
                    target_pins = Path(fixture_dir) / 'target-selected.json'
                    target_pins.write_text(json.dumps({'embedding':tiers_manifest['embedding'],'chat':answers[1]}))
                    with Path('installed-migration-target-check.log').open('w',encoding='utf-8') as log:
                        checked = subprocess.run([str(cli),'--cpu','--state',str(legacy),'--assets',str(seed_models),'--models',str(target_pins),'ask','What is the orbital archive retention policy?'],stdout=log,stderr=subprocess.STDOUT,timeout=240,check=False)
                        assert checked.returncode == 0, 'Target verify-once preparation failed'
                    migration_env['VZ_MIGRATION_SMOKE'] = 'switch'
                    with Path('installed-migration-switch.log').open('w',encoding='utf-8') as log:
                        child = subprocess.Popen([str(binary),'--pipeline-smoke-test','--cpu'],cwd=fixture_dir,env=migration_env,stdout=log,stderr=subprocess.STDOUT)
                        try:
                            assert child.wait(timeout=360) == 0, 'Installed migration switch/delete failed'
                        finally:
                            if child.poll() is None:
                                child.kill(); child.wait(timeout=10)
                    assert not (seed_models / answers[0]['file']).exists(), 'Unused old model was not deleted'
        with tempfile.TemporaryDirectory(prefix='vz-import-three-') as import_dir:
            import_root = Path(import_dir)
            source = import_root / 'source'
            source.mkdir()
            for pin in [tiers_manifest['embedding'], *answers]:
                original = second_file if pin['sha256'] == second['sha256'] else models / pin['file']
                shutil.copy2(original, source / pin['file'])
            import_env = dict(env, VZ_SMOKE_DATA_ROOT=str(import_root / 'library'),
                              WEBVIEW2_USER_DATA_FOLDER=str(import_root / 'webview2'),
                              VZ_SMOKE_MODELS=str(Path('spike/fixtures/models.tiers.json').resolve()),
                              VZ_PIPELINE_FIXTURE=str(Path('spike/fixtures/documents').resolve()),
                              VZ_SMOKE_IMPORT_FOLDER=str(source), VZ_MIGRATION_SMOKE='import')
            with Path('installed-import-three.log').open('w',encoding='utf-8') as log:
                child = subprocess.Popen([str(binary),'--pipeline-smoke-test','--cpu'],cwd=fixture_dir,env=import_env,stdout=log,stderr=subprocess.STDOUT)
                try:
                    assert child.wait(timeout=480) == 0, 'Installed three-pin import failed'
                finally:
                    if child.poll() is None:
                        child.kill(); child.wait(timeout=10)
    step('Installed migrations kept selection, model stamps and the seeded index across restarts; switch/delete passed')
    step('Check persistent data sentinel before uninstall')
    sentinel = data / 'install-self-test.json'
    before = sentinel.read_bytes()
finally:
    step('Uninstall; preserve user data')
    if windows:
        run(msiexec, '/x', installer, '/qn', '/norestart',
            '/l*v', str(Path('msi-uninstall.log').resolve()), timeout=120)
    else:
        shutil.rmtree(installed)
assert not binary.exists(), 'Uninstall left the executable installed'
assert sentinel.read_bytes() == before, 'Uninstall removed or changed user data'
for path, digest in model_hashes.items():
    with path.open('rb') as stream:
        assert hashlib.file_digest(stream, 'sha256').hexdigest() == digest, 'Uninstall changed model data'
print(f'PASS: {platform.platform()} hosted-runner stand-in: installed UI self-test, model acquisition, pipeline and double launch; uninstall preserved {data}')
