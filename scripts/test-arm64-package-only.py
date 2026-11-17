"""Pure memory mocks for the ARM64 package-only entry point; no payload runs."""
import ast
import json
from pathlib import Path, PurePosixPath
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock


class PackageOnlyTests(unittest.TestCase):
    def context(self, fail_build=False):
        source = Path(__file__).with_name('check-arm64-package.py').read_text(encoding='utf-8')
        main = next(n for n in ast.parse(source).body if isinstance(n, ast.FunctionDef) and n.name == 'main')
        storage = {'/project/obz-gui/tauri.conf.json': '{"version":"0.11.23"}',
                   '/dist/qing-zip-0.11.23-arm64-portable.zip': 'portable package bytes'}

        class MemoryPath:
            def __init__(self, value): self.value = str(value)
            def __str__(self): return self.value
            def __truediv__(self, name): return MemoryPath(self.value+'/'+str(name))
            @property
            def name(self): return PurePosixPath(self.value).name
            @property
            def stem(self): return PurePosixPath(self.value).stem
            def resolve(self): return self
            def is_file(self): return self.value in storage
            def exists(self): return self.value in storage
            def read_text(self): return storage[self.value]
            def write_text(self, value, encoding=None): storage[self.value] = value
            def stat(self): return SimpleNamespace(st_size=len(storage[self.value]))
            def glob(self, pattern): return iter([self/'nsis-3.13/makensis.exe'])

        parser = MagicMock()
        parser.parse_args.return_value = SimpleNamespace(dist=MemoryPath('/dist'), downloads=MemoryPath('/downloads'), package_only=True)
        temporary = MagicMock()
        temporary.__enter__.return_value = '/temporary'
        forbidden = {name: MagicMock(side_effect=AssertionError('forbidden call: '+name))
                     for name in ['exercise', 'block_delete', 'key', 'files', 'verify_retry', 'verify_foreign_shortcuts', 'seed_runner_associations']}
        def compile_installer(arguments):
            self.assertEqual(str(arguments[0]), '/temporary/nsis/nsis-3.13/makensis.exe')
            self.assertFalse(any(str(argument) == '/S' for argument in arguments))
            if fail_build: raise RuntimeError('mock compiler failure')
            output = next(str(arg).removeprefix('/DOUTPUT=') for arg in arguments if str(arg).startswith('/DOUTPUT='))
            storage[output] = 'installer package bytes'
        context = dict(Path=MemoryPath, ROOT=MemoryPath('/project'), json=json,
                       argparse=SimpleNamespace(ArgumentParser=lambda:parser),
                       tempfile=SimpleNamespace(TemporaryDirectory=lambda:temporary),
                       zipfile=SimpleNamespace(ZipFile=MagicMock()), fetch=MagicMock(return_value='NSIS package'), NSIS=('url','sha'),
                       run=MagicMock(side_effect=compile_installer), machine=MagicMock(return_value=0x14c),
                       sha=MagicMock(return_value='a'*64), os=SimpleNamespace(environ={}), print=MagicMock(), **forbidden)
        exec(compile(ast.Module(body=[main], type_ignores=[]), 'actual-package-main', 'exec'), context)
        return context, storage, forbidden

    def test_package_only_builds_hashes_and_returns_before_acceptance(self):
        context, storage, forbidden = self.context()
        context['main']()
        context['run'].assert_called_once()
        for mock in forbidden.values(): mock.assert_not_called()
        report=json.loads(storage['/dist/qing-zip-0.11.23-arm64-package-build.json'])
        self.assertFalse(report['installed'])
        self.assertFalse(report['functional_acceptance_run'])
        self.assertFalse(report['native_filesystem_tests_run'])
        self.assertEqual(len(report['assets']), 2)
        self.assertNotIn('/dist/qing-zip-0.11.23-arm64-verification.json', storage)

    def test_failed_mock_compilation_does_not_emit_a_package_receipt(self):
        context, storage, forbidden = self.context(fail_build=True)
        with self.assertRaisesRegex(RuntimeError, 'mock compiler failure'): context['main']()
        for mock in forbidden.values(): mock.assert_not_called()
        self.assertFalse(any(name.endswith('-package-build.json') for name in storage))


if __name__ == '__main__': unittest.main()
