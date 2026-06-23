# Archive test fixtures

PMarc (`.pma`) archives used to validate the `archive::pma` decoders.
Taken from Simon Howard's `lhasa` test suite (https://github.com/fragglet/lhasa),
which is ISC-licensed. Each archive carries an embedded CRC-16 that the tests
check the decompressed output against.
