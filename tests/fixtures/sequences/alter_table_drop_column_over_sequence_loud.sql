-- Same `ATSimplePermissions` gate on the DROP COLUMN action: the relkind is refused
-- before the column is looked for.
-- @setup
CREATE SEQUENCE atd_seq;
-- @test
ALTER TABLE atd_seq DROP COLUMN nosuchcol;
-- @expect-error: ALTER TABLE cannot be performed on a sequence (42809)
