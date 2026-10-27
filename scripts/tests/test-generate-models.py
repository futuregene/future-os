"""Offline regression tests for the actual catalog generator."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "generate_models", (Path(__file__).resolve().parents[1] / "docs" / "generate_models.py")
)
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


class CatalogTests(unittest.TestCase):
    def test_inspected_source_samples_match_the_actual_shipped_catalog(self):
        root = Path(__file__).resolve().parents[2]
        evidence = json.loads((root / "docs/archives/bughunt/model-output-provenance-20260911.json").read_text(encoding="utf-8"))
        catalog = json.loads((root / "agent/src/models/builtin/models.json").read_text(encoding="utf-8"))
        by_key = {(model["provider"], model["id"]): model for model in catalog}
        for sample in evidence["samples"]:
            with self.subTest(provider=sample["provider"], model=sample["id"]):
                self.assertEqual(generator.model_modalities(sample["source_fields"], "output"), sample["expected_output"])
                self.assertEqual(by_key[sample["provider"], sample["id"]]["output"], sample["expected_output"])

    def test_modalities_survive_each_upstream_and_generated_schema(self):
        # Intentionally opaque ids: classification must use metadata, not names.
        models_dev = generator.process_models_dev({
            "fixture": {"api": "https://example.invalid", "models": {
                "opaque-1": {"tool_call": True, "modalities": {"input": ["text", "image"], "output": ["image"]}},
                "opaque-2": {"tool_call": True, "modalities": {"output": ["text", "image"]}},
                "opaque-3": {"tool_call": True, "modalities": {"output": []}},
            }},
        })
        openrouter = generator.process_openrouter({"data": [{
            "id": "opaque-4", "supported_parameters": ["tools"],
            "architecture": {"input_modalities": ["text", "image"], "output_modalities": ["image"]},
        }, {
            "id": "opaque-5", "supported_parameters": ["tools"],
            "architecture": {"modality": "text+image->text+audio"},
        }]})
        gateway = generator.process_vercel_ai({"data": [
            {"id": "opaque-6", "type": "embedding", "modalities": {"input": ["text"], "output": ["text"]}},
            {"id": "opaque-7", "type": "video"},
            {"id": "opaque-8", "architecture": {"output_modalities": ["image"]}},
            {"id": "opaque-9", "type": "language"},
        ]})
        generated = json.loads(generator.generate_models_json(models_dev + openrouter + gateway))
        by_id = {model["id"]: model for model in generated}
        expected = {
            "opaque-1": ["image"], "opaque-2": ["text", "image"], "opaque-3": [],
            "opaque-4": ["image"], "opaque-5": ["text", "audio"],
            "opaque-6": ["embedding"], "opaque-7": ["video"], "opaque-8": ["image"], "opaque-9": ["text"],
        }
        for model_id, output in expected.items():
            with self.subTest(model_id=model_id):
                self.assertEqual(by_id[model_id]["output"], output)
        self.assertEqual(by_id["opaque-1"]["input"], ["text", "image"])
        self.assertEqual(by_id["opaque-4"]["input"], ["text", "image"])
        self.assertEqual(by_id["opaque-5"]["input"], ["text", "image"])

    def test_absent_output_keeps_legacy_default_but_empty_output_stays_empty(self):
        models = generator.process_vercel_ai({"data": [{"id": "legacy"}]})
        models[0].pop("output")
        self.assertEqual(json.loads(generator.generate_models_json(models))[0]["output"], ["text"])
        models[0]["output"] = []
        self.assertEqual(json.loads(generator.generate_models_json(models))[0]["output"], [])

    def test_failed_or_empty_sources_preserve_catalog_and_docs(self):
        for response in (None, {}):
            with self.subTest(response=response), tempfile.TemporaryDirectory() as directory:
                catalog = Path(directory) / "models.json"
                catalog.write_text('[{"id":"existing"}]')
                with patch.object(generator, "fetch_json", return_value=response), \
                     patch.object(generator, "repo_path", return_value=catalog), \
                     patch.object(generator, "generate_wiki_docs") as wiki:
                    with self.assertRaisesRegex(RuntimeError, "refusing to overwrite"):
                        generator.main()
                    self.assertEqual(catalog.read_text(), '[{"id":"existing"}]')
                    wiki.assert_not_called()

    def test_thinking_levels_follow_the_reasoning_flag_and_overrides(self):
        self.assertEqual(
            generator.thinking_levels_for("openai", "gpt-5", True),
            ["minimal", "low", "medium", "high"],
        )
        self.assertEqual(generator.thinking_levels_for("openai", "gpt-4o", False), [])

        with patch.dict(generator.PROVIDER_THINKING_LEVELS, {"fixture": ["low", "max"]}):
            self.assertEqual(
                generator.thinking_levels_for("fixture", "m", True), ["low", "max"]
            )
            with patch.dict(
                generator.MODEL_THINKING_LEVELS, {"fixture/m": ["off", "high"]}
            ):
                self.assertEqual(
                    generator.thinking_levels_for("fixture", "m", True), ["off", "high"]
                )
                # A model override still cannot resurrect a non-reasoning model.
                self.assertEqual(generator.thinking_levels_for("fixture", "m", False), [])

    def test_generated_catalog_only_carries_declared_thinking_levels(self):
        reasoning = {"id": "r", "name": "R", "provider": "p", "reasoning": True,
                     "context_window": 1000, "max_tokens": 100, "cost_input": 0,
                     "cost_output": 0, "cost_cache_read": 0, "cost_cache_write": 0}
        plain = dict(reasoning, id="p", name="P", reasoning=False)
        generated = json.loads(generator.generate_models_json([reasoning, plain]))
        by_id = {model["id"]: model for model in generated}
        self.assertEqual(by_id["r"]["thinking_levels"], ["minimal", "low", "medium", "high"])
        self.assertNotIn("thinking_levels", by_id["p"])
        self.assertNotIn("tlm_json", by_id["r"])

    def test_backfill_rewrites_only_the_thinking_columns(self):
        with tempfile.TemporaryDirectory() as directory:
            catalog = Path(directory) / "models.json"
            catalog.write_text(json.dumps([
                {"id": "r", "provider": "p", "reasoning": True, "tlm_json": "{}",
                 "compat_json": "{}", "headers_json": "{}", "hide": False},
                {"id": "p", "provider": "p", "reasoning": False, "tlm_json": "{}",
                 "compat_json": "{}", "headers_json": "{}", "hide": False},
            ]))
            self.assertEqual(generator.backfill_thinking_levels(str(catalog)), 2)
            first = json.loads(catalog.read_text())
            self.assertEqual(first[0]["thinking_levels"], ["minimal", "low", "medium", "high"])
            self.assertNotIn("thinking_levels", first[1])
            self.assertNotIn("tlm_json", first[0])
            self.assertEqual(first[0]["compat_json"], "{}")
            # Idempotent.
            generator.backfill_thinking_levels(str(catalog))
            self.assertEqual(json.loads(catalog.read_text()), first)


if __name__ == "__main__":
    unittest.main()
