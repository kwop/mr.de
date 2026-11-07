"""Guard preview isolation and truthful capture failures without launching GPUI."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ui_catalog


class CatalogSafetyTests(unittest.TestCase):
    story = {"id": "reader.test", "scenario": "focus:description"}

    def executable(self, directory, source):
        script = directory / "fake-preview"
        script.write_text(f"#!{sys.executable}\n" + source)
        script.chmod(0o700)
        return script

    def test_preview_discards_real_data_overrides(self):
        with patch.dict(os.environ, {
            "MRDE_SNAPSHOT_REAL": "1", "MRDE_SCENARIO_RISK_FILE": "/private/report.json",
            "MRDE_SCENARIO": "settings", "MRDE_SNAPSHOT": "/private/capture.png",
            "MRDE_TERMINAL_NAVIGATE": "1",
        }):
            env = ui_catalog.preview_environment(self.story)
        self.assertEqual(env["MRDE_SCENARIO"], "focus:description")
        self.assertEqual({key for key in env if key.startswith("MRDE_")},
                         {"MRDE_SCENARIO", "MRDE_WINDOW"})
        self.assertIn("PATH", env)

    def test_existing_capture_directory_is_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            reference = Path(tmp) / "accepted.png"
            reference.write_bytes(b"reference")
            with self.assertRaises(FileExistsError):
                ui_catalog.new_capture_directory(tmp)
            self.assertEqual(reference.read_bytes(), b"reference")

    def test_successful_process_without_capture_is_a_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            binary = self.executable(directory, "import sys\nassert sys.argv[1:] == ['--ui-preview']\n")
            with self.assertRaisesRegex(ValueError, "produced no capture"):
                ui_catalog.run_capture(binary, self.story, directory, 5)

    def test_timeout_reaps_the_started_process(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            pid_file = directory / "pid"
            binary = self.executable(directory,
                "import os, time\nfrom pathlib import Path\n"
                f"Path({str(pid_file)!r}).write_text(str(os.getpid()))\n"
                "time.sleep(30)\n")
            with self.assertRaises(subprocess.TimeoutExpired):
                ui_catalog.run_capture(binary, self.story, directory, 1)
            pid = int(pid_file.read_text())
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)


if __name__ == "__main__":
    unittest.main()
