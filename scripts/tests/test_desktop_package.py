import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


class DesktopPackageContractTests(unittest.TestCase):
    def test_batch_builds_a_portable_windows_zip_with_its_sidecar(self):
        script = (ROOT / "scripts" / "build-desktop.bat").read_text(encoding="utf-8").lower()
        for required in (
            "where.exe node",
            "where.exe npm",
            "where.exe cargo",
            "x86_64-pc-windows-msvc",
            "cargo build --release -p agent-desktop --features mcp-streamable-http",
            "agent-desktop-x86_64-pc-windows-msvc.exe",
            "npm ci",
            "--no-bundle",
            "agent-desktop-mcp-console.exe",
            "agent-desktop.exe",
            "compress-archive",
            "releases\\agent-desktop-mcp-portable_",
        ):
            with self.subTest(required=required):
                self.assertIn(required.lower(), script)
        self.assertNotIn("--bundles nsis", script)

    def test_tauri_declares_the_sidecar_next_to_the_portable_app(self):
        config = json.loads((ROOT / "apps" / "desktop" / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8"))
        self.assertEqual(["binaries/agent-desktop"], config["bundle"]["externalBin"])

    def test_docs_include_desktop_setup_in_existing_mcp_navigation(self):
        guide = (ROOT / "apps" / "docs" / "content" / "docs" / "guides" / "mcp.mdx").read_text(encoding="utf-8")
        nav = json.loads((ROOT / "apps" / "docs" / "content" / "docs" / "guides" / "meta.json").read_text(encoding="utf-8"))
        self.assertIn("mcp", nav["pages"])
        self.assertIn("http://127.0.0.1:12899/desktop", guide)
        self.assertIn('"type": "streamable-http"', guide)
        self.assertIn("portable", guide.lower())
        self.assertIn("agent-desktop-mcp-console.exe", guide)
        self.assertIn("releases/agent-desktop-mcp-portable_", guide)


if __name__ == "__main__":
    unittest.main()
