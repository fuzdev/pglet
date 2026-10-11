-- `pg_column_size` is `STABLE`, so an index expression over it is `42P17` on both.
-- @setup
CREATE TABLE pci (b bytea);
-- @test
-- @expect-error: functions in index expression must be marked IMMUTABLE
CREATE INDEX pci_size ON pci ((pg_column_size(b)));
