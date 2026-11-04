#!/usr/bin/env bash
# Downloads the public FML maps used by the corpus test into ./corpus (not committed).
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p corpus
fetch() {
  local name=$1 url=$2
  if [ -d "corpus/$name/.git" ]; then
    git -C "corpus/$name" pull -q --ff-only
  else
    git clone -q --depth 1 "$url" "corpus/$name"
  fi
}
fetch mapping-language-ig https://github.com/HL7/mapping-language-ig
fetch fhir-cross-version https://github.com/HL7/fhir-cross-version
fetch fhir-r6-maps https://github.com/brianpos/fhir-r6-maps
fetch fhir-test-cases https://github.com/FHIR/fhir-test-cases
# HL7 C-CDA examples (CC0), for the XML round trip of fml-xml.
fetch C-CDA-Examples https://github.com/HL7/C-CDA-Examples
# The R4 specification examples, for the cross-version conversion test.
if [ ! -d corpus/r4-examples ]; then
  mkdir -p corpus/r4-examples
  curl -sfL -o corpus/r4-examples.zip https://hl7.org/fhir/R4/examples-json.zip
  unzip -q corpus/r4-examples.zip -d corpus/r4-examples
  rm corpus/r4-examples.zip
fi
# The R5 specification examples: what the converted R4 examples are compared with.
if [ ! -d corpus/r5-examples ]; then
  mkdir -p corpus/r5-examples
  curl -sfL -o corpus/r5-examples.zip https://hl7.org/fhir/R5/examples-json.zip
  unzip -q corpus/r5-examples.zip -d corpus/r5-examples
  rm corpus/r5-examples.zip
fi
find corpus \( -name '*.fml' -o -name '*.map' \) -not -path '*/node_modules/*' | wc -l | xargs echo "maps:"
