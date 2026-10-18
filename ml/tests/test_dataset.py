import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from asmr_ml.dataset import prepared_files, publish_prepared


class PreparedDatasetTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)

    def report(self, directory, records):
        (directory / "prep.json").write_text(json.dumps(records))

    def test_rejected_stale_files_never_enter_training(self):
        (self.directory / "old.npz").write_bytes(b"stale")
        (self.directory / "new.npz").write_bytes(b"accepted")
        self.report(self.directory, [{"id": "old", "skip": "snr"}, {"id": "new", "file": "new.npz"}])
        self.assertEqual(prepared_files(self.directory), [self.directory / "new.npz"])

    def test_publish_removes_files_rejected_by_stricter_preparation(self):
        (self.directory / "old.npz").write_bytes(b"old")
        self.report(self.directory, [{"file": "old.npz"}])
        staging = self.directory / "staging"
        staging.mkdir()
        self.report(staging, [{"skip": "snr below threshold"}])
        publish_prepared(staging, self.directory)
        self.assertEqual(prepared_files(self.directory), [])
        self.assertFalse((self.directory / "old.npz").exists())

    def test_interrupted_publication_keeps_previous_dataset(self):
        (self.directory / "old.npz").write_bytes(b"old")
        self.report(self.directory, [{"file": "old.npz"}])
        staging = self.directory / "staging"
        staging.mkdir()
        (staging / "generation-new.npz").write_bytes(b"new")
        self.report(staging, [{"file": "generation-new.npz"}])
        replace = os.replace

        def interrupt(source, destination):
            if Path(source).name == "prep.json":
                raise OSError("publication interrupted")
            replace(source, destination)

        with patch("asmr_ml.dataset.os.replace", side_effect=interrupt):
            with self.assertRaises(OSError):
                publish_prepared(staging, self.directory)
        self.assertEqual(prepared_files(self.directory), [self.directory / "old.npz"])
        self.assertEqual((self.directory / "old.npz").read_bytes(), b"old")

    def test_missing_manifest_is_an_error_instead_of_training_on_orphans(self):
        (self.directory / "orphan.npz").touch()
        with self.assertRaises(FileNotFoundError):
            prepared_files(self.directory)


if __name__ == "__main__":
    unittest.main()
