-include config/.generated.env config/.env config/.local.env
export

SPEC   ?= examples/full/Vedfile
TARGET ?= $(or $(DEPLOYMENT_TARGET),compose)
OUTPUT ?= $(dir $(SPEC))

.PHONY: all build compile unit-test up down reset clean ps e2e demo smoke-test release sonar diff-verify

all: build

## release — bump version, test, tag, push and publish (usage: make release [VERSION=patch|minor|major|x.y.z])
release:
	@bash scripts/release.sh $(VERSION)

## demo — run complete showcase: compile, test, start stack, verify health
demo: up ps e2e

## build — compile spec and run unit tests
build: compile unit-test

## compile — generate deployment manifests
compile:
	@./vew compile -f $(SPEC) -t $(TARGET) -o $(OUTPUT)

## unit-test — run unit test suite
unit-test:
	@cargo test -q
	@.venv/bin/pytest tests/ -q

## up — start stack using ved native CLI
up:
	@./vew up -f $(SPEC) -t $(TARGET)

## down — stop running services
down:
	@./vew down -t $(TARGET)

## reset — tear down all resources
reset:
	@./vew reset -t $(TARGET)

## ps — display runtime status
ps:
	@./vew ps -t $(TARGET)

## smoke-test — run in-cluster verification
smoke-test:
	@./vew test -t $(TARGET)

## e2e — run end-to-end healthcheck against lambda-emulator
e2e:
	@bash scripts/targets/e2e.sh

## sonar — run SonarQube code quality scan
sonar:
	@bash scripts/sonar-scanner.sh

## diff-verify — verify compiled manifest canonical runtime equivalence
diff-verify:
	@./vew compile -f $(SPEC) -t compose -o $(OUTPUT)
	@python3 scripts/canonical-diff.py --source $(OUTPUT)/docker-compose.yml --compiled $(OUTPUT)/docker-compose.yml

## clean — remove caches and temporary build artifacts
clean:
	@find . -type d -name __pycache__ -exec rm -rf {} + 2>/dev/null || true
	@find . -name "*.pyc" -delete 2>/dev/null || true
	@rm -rf .pytest_cache

