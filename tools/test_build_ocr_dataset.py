import json
import tempfile
import unittest
from pathlib import Path

from build_ocr_dataset import collect, iter_items


class DumpLayoutTests(unittest.TestCase):
    def make_item(self, root, game="genshin"):
        item = root / "characters" / "0000"
        item.mkdir(parents=True)
        (item / "name.png").write_bytes(b"fixture crop")
        (item / "failed.png").write_bytes(b"fixture crop")
        (item / "ocr_fields.json").write_text(json.dumps({
            "game": game,
            "final_object": None,
            "fields": [
                {"field": "name", "crop": "name.png", "raw": "verified later"},
                {"field": "name", "crop": "failed.png", "raw": "ERROR: failed", "inference_error": True},
            ],
        }), encoding="utf-8")

    def test_new_runs_are_independent_and_hsr_is_excluded(self):
        with tempfile.TemporaryDirectory() as temp:
            cwd = Path(temp)
            first = cwd / "debug_images" / "genshin" / "run_1"
            second = cwd / "debug_images" / "genshin" / "run_2"
            for root in [first, second]:
                self.make_item(root)
            hsr = cwd / "debug_images" / "hsr" / "run_1"
            self.make_item(hsr, "hsr")
            records = collect([str(cwd)], {}, None)
            self.assertEqual(len(records), 2)
            self.assertEqual(len({r["run"] for r in records}), 2)
            self.assertTrue(all("genshin" in r["src_path"] for r in records))
            self.assertEqual(len(list(iter_items(str(first)))), 1)
            self.assertEqual(collect([str(hsr)], {}, None), [])

    def test_historical_flat_layout_stays_readable(self):
        with tempfile.TemporaryDirectory() as temp:
            cwd = Path(temp)
            self.make_item(cwd / "debug_images")
            self.assertEqual(len(collect([str(cwd)], {}, None)), 1)


if __name__ == "__main__":
    unittest.main()
