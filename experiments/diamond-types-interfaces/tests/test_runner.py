# SPDX-License-Identifier: Apache-2.0

"""Safety contracts for the disposable source-tree generator."""

import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("interface_spike", ROOT / "experiments/diamond-types-interfaces/run.py")
SPIKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SPIKE)


def archive(name, symlink=False):
    result = io.BytesIO()
    with tarfile.open(fileobj=result, mode="w") as bundle:
        member = tarfile.TarInfo(name)
        if symlink:
            member.type = tarfile.SYMTYPE
            member.linkname = "../outside"
            bundle.addfile(member)
        else:
            member.size = 3
            bundle.addfile(member, io.BytesIO(b"abc"))
    return result.getvalue()


class ArchiveTests(unittest.TestCase):
    def test_regular_nested_file_is_copied_exactly(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as folder:
            SPIKE.extract_archive(archive("src/lib.rs"), Path(folder))
            self.assertEqual((Path(folder) / "src/lib.rs").read_bytes(), b"abc")

    def test_absolute_and_parent_paths_are_rejected(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as folder:
            for name in ("/absolute.rs", "../outside.rs", "src/../../outside.rs"):
                with self.subTest(name=name), self.assertRaises(ValueError):
                    SPIKE.extract_archive(archive(name), Path(folder))

    def test_windows_escape_paths_are_rejected_on_every_host(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as folder:
            for name in ("C:/outside.rs", "src\\..\\..\\outside.rs"):
                with self.subTest(name=name), self.assertRaises(ValueError):
                    SPIKE.extract_archive(archive(name), Path(folder))

    def test_symlinks_cannot_escape_the_generated_tree(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as folder:
            with self.assertRaises(ValueError):
                SPIKE.extract_archive(archive("link", symlink=True), Path(folder))


if __name__ == "__main__":
    unittest.main()
