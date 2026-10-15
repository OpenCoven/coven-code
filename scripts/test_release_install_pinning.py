#!/usr/bin/env python3
"""Execute generated install snippets with inert, offline installer responses."""
import ast
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("regen-release-notes.py")


def install_section(tag):
    # Importing the module creates /tmp/release-notes; execute only its real function.
    tree = ast.parse(SCRIPT.read_text())
    function = next(node for node in tree.body
                    if isinstance(node, ast.FunctionDef) and node.name == "install_section")
    namespace = {}
    exec(compile(ast.Module(body=[function], type_ignores=[]), str(SCRIPT), "exec"), namespace)
    return namespace["install_section"](tag)


class ReleaseInstallPinning(unittest.TestCase):
    def test_bash_installer_receives_release_version(self):
        bash = shutil.which("bash")
        if not bash:
            self.skipTest("bash unavailable")
        for tag in ("v0.0.1", "v0.0.17"):
            with self.subTest(tag=tag), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / "bash").symlink_to(bash)
                curl = root / "curl"
                curl.write_text('#!' + bash + '\n'
                                'printf "%s\\n" "$@" > "$REQUEST_LOG"\n'
                                'printf \'printf "ARG:<%%s>\\\\n" "$@"\\n\'\n')
                curl.chmod(0o700)
                snippet = re.findall(r"```bash\n(.*?)\n```", install_section(tag), re.S)[0]
                result = subprocess.run([bash, "-c", snippet], text=True, capture_output=True,
                                        timeout=10, env={"HOME": temporary, "PATH": temporary,
                                                         "REQUEST_LOG": str(root / "request")})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual((root / "request").read_text().splitlines(),
                                 ["-fsSL", f"https://github.com/OpenCoven/coven-code/releases/download/{tag}/install.sh"])
                self.assertEqual(result.stdout.splitlines(), ["ARG:<--version>", f"ARG:<{tag[1:]}>"])

    def test_powershell_installer_receives_release_version(self):
        pwsh = shutil.which("pwsh")
        if not pwsh:
            self.skipTest("pwsh unavailable")
        for tag in ("v0.0.1", "v0.0.17"):
            with self.subTest(tag=tag), tempfile.TemporaryDirectory() as temporary:
                snippet = re.search(r"```powershell\n(.*?)\n```", install_section(tag), re.S)[1]
                script = Path(temporary) / "fixture.ps1"
                script.write_text('''$ErrorActionPreference = 'Stop'
Import-Module Microsoft.PowerShell.Utility -ErrorAction Stop
$PSModuleAutoLoadingPreference = 'None'
function Invoke-RestMethod { throw 'Unexpected real HTTP request' }
function Invoke-WebRequest { throw 'Unexpected real HTTP request' }
function Invoke-FixtureInstaller([string]$Uri) {
    [IO.File]::WriteAllText($env:REQUEST_LOG, $Uri)
    return 'param([string]$Version) Write-Output "VERSION:<$Version>"'
}
Set-Alias -Name irm -Value Invoke-FixtureInstaller -Scope Script -Force
if ((Get-Command irm).CommandType -ne 'Alias' -or
    (Get-Command irm).Definition -ne 'Invoke-FixtureInstaller' -or
    (Get-Command Invoke-FixtureInstaller).CommandType -ne 'Function') {
    throw 'Offline installer mock resolution failed'
}
''' + snippet + '\n')
                result = subprocess.run([pwsh, "-NoProfile", "-NonInteractive", "-File", str(script)],
                                        text=True, capture_output=True, timeout=45,
                                        env={"HOME": temporary, "PATH": temporary,
                                             "REQUEST_LOG": str(Path(temporary) / "request")})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual((Path(temporary) / "request").read_text(),
                                 f"https://github.com/OpenCoven/coven-code/releases/download/{tag}/install.ps1")
                self.assertEqual(result.stdout.splitlines(), [f"VERSION:<{tag[1:]}>"])

    def test_existing_pinned_alternatives(self):
        section = install_section("v0.0.17")
        self.assertIn("npm install -g @opencoven/coven-code@0.0.17", section)
        self.assertIn("coven-code upgrade --version 0.0.17", section)
        self.assertIn("coven-code --version", section)


if __name__ == "__main__":
    unittest.main()
