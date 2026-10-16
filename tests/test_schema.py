"""Tests validating ved.yaml against schemas/ved.schema.json."""

import json
from pathlib import Path
import pytest
import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent
SCHEMA_PATH = REPO_ROOT / "schemas" / "ved.schema.json"
VED_YAML_PATH = REPO_ROOT / "examples" / "full" / "ved.yaml"


@pytest.fixture
def schema():
    with open(SCHEMA_PATH, encoding="utf-8") as f:
        return json.load(f)


@pytest.fixture
def spec_data():
    with open(VED_YAML_PATH, encoding="utf-8") as f:
        return yaml.safe_load(f)


def test_ved_yaml_validates(schema, spec_data):
    """The workspace ved.yaml must strictly pass schema validation."""
    from jsonschema import validate
    validate(instance=spec_data, schema=schema)


def test_schema_rejects_unknown_top_level_property(schema, spec_data):
    """Schema must reject unexpected top-level keys."""
    from jsonschema import validate, ValidationError
    bad_data = dict(spec_data)
    bad_data["unknown_key"] = "forbidden"
    with pytest.raises(ValidationError):
        validate(instance=bad_data, schema=schema)


def test_schema_rejects_missing_required_fields(schema, spec_data):
    """Schema must require app, version, and services."""
    from jsonschema import validate, ValidationError
    bad_data = dict(spec_data)
    del bad_data["app"]
    with pytest.raises(ValidationError):
        validate(instance=bad_data, schema=schema)


def test_schema_rejects_invalid_port(schema, spec_data):
    """Schema must reject negative or out-of-range ports."""
    from jsonschema import validate, ValidationError
    bad_data = dict(spec_data)
    bad_data["services"]["postgres"]["port"] = 999999
    with pytest.raises(ValidationError):
        validate(instance=bad_data, schema=schema)


def test_schema_rejects_invalid_hook(schema, spec_data):
    """Schema must reject unsupported hook types."""
    from jsonschema import validate, ValidationError
    bad_data = dict(spec_data)
    bad_data["jobs"]["smoke-test"]["hook"] = "invalid-hook"
    with pytest.raises(ValidationError):
        validate(instance=bad_data, schema=schema)
