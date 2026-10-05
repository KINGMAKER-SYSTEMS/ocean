# ocean-agents — developer targets.
#
# The assistants/<SURFACE>/system.md profiles the Ocean daemon loads are COMPOSED
# artifacts (assistants/tools/compose_profile.py). Do not hand-edit them — edit the
# _shared/ + _base/<SURFACE>/ sources and re-compose. These targets make that flow
# one command and give CI / pre-commit a single drift gate.

PY ?= python3
COMPOSER := assistants/tools/compose_profile.py
# Surfaces with a _base/<SURFACE>/ source dir (the ones the composer publishes).
SURFACES := $(notdir $(wildcard assistants/_base/*))

.PHONY: assistants assistants-check assistants-compose help

help:
	@echo "make assistants-compose  - regenerate every published assistants/<SURFACE>/system.md from its _base sources"
	@echo "make assistants-check    - verify every published profile matches its _base sources (drift gate); fails on drift"
	@echo "make assistants          - alias for assistants-check"

# Re-publish every surface profile from its sources.
assistants-compose:
	@for s in $(SURFACES); do $(PY) $(COMPOSER) $$s --write; done

# Drift gate: every published profile must match its sources AND be non-empty.
# `--check` with no surface checks all surfaces and exits non-zero on drift.
assistants-check:
	@$(PY) -m py_compile $(COMPOSER)
	@$(PY) $(COMPOSER) --check

assistants: assistants-check
