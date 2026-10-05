import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location('scope', pathlib.Path(__file__).with_name('build-scope.py'))
scope = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scope)


class BuildScopeTests(unittest.TestCase):
    def test_docs_do_not_compile(self):
        self.assertEqual(scope.build_scope(['AGENTS.md', 'events.md', 'platform/ocean-os/docs/ARCHITECTURE.md', 'apps/ocean-surface/AGENTS.md']), (False, False))

    def test_components_are_independent(self):
        self.assertEqual(scope.build_scope(['platform/ocean-os/crates/ocean-providers/src/lib.rs']), (True, False))
        self.assertEqual(scope.build_scope(['apps/ocean-surface/vscode-extension/package-lock.json']), (False, True))

    def test_workflow_and_shared_inputs_build_both(self):
        for path in ['.github/workflows/ci.yml', '.github/build-scope.py', '.cargo/config.toml', 'Cargo.lock', 'rust-toolchain.toml']:
            self.assertEqual(scope.build_scope([path]), (True, True), path)

    def test_embedded_markdown_is_a_build_input(self):
        self.assertEqual(scope.build_scope(['platform/ocean-os/crates/ocean-agent/src/prompt.md']), (True, False))

    def test_both_components_and_deleted_inputs(self):
        self.assertEqual(scope.build_scope(['platform/ocean-os/Cargo.lock', 'apps/ocean-surface/Trunk.toml']), (True, True))

    def test_packages_and_internal_sops_do_not_compile(self):
        self.assertEqual(scope.build_scope(['packages/ocean-agents/assistants/bonzai/profile.json', 'org/risingtides-agents/docs/plan.md']), (False, False))


if __name__ == '__main__':
    unittest.main()
