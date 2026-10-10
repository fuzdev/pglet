-- A `char(n)` column enforces the same length cap as `varchar(n)` (pglet maps both
-- to one length-bearing type): `'abcd'` into `char(3)` fails loud (SQLSTATE 22001).
-- pglet does not model bpchar blank-padding, so only the over-length error case is
-- asserted here — the SQLSTATE matches Postgres.
-- @setup
CREATE TABLE co (v char(3));
-- @test
-- @expect-error: value too long for type character(3)
INSERT INTO co VALUES ('abcd');
