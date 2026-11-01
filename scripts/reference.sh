#!/usr/bin/env bash
# Compiles the corpus with the HL7 reference compiler (org.hl7.fhir.core, via validator_cli.jar)
# into reference/, for the reference test: cargo test -p fml-model --test reference.
# Needs Java 17+ and the corpus (scripts/fetch-corpus.sh). Set VALIDATOR_JAR to reuse a jar.
set -euo pipefail
cd "$(dirname "$0")/.."
jar=${VALIDATOR_JAR:-reference/validator_cli.jar}
if [ ! -f "$jar" ]; then
  mkdir -p "$(dirname "$jar")"
  curl -sfL -o "$jar" https://github.com/hapifhir/org.hl7.fhir.core/releases/latest/download/validator_cli.jar
fi
roots=${FML_CORPUS:-corpus}
IFS=: read -ra dirs <<< "$roots"
find "${dirs[@]}" \( -name '*.fml' -o -name '*.map' \) -not -path '*/node_modules/*' | sort \
  | java -cp "$jar" scripts/reference/CompileAll.java reference/maps 2>/dev/null | grep -v '^FML:'
