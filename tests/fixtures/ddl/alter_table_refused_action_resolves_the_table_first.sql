-- Without `IF EXISTS`, a missing table is `42P01` ahead of the refusal of an action pglet
-- does not perform — 18.6 looks the relation up before it analyses the action. The
-- refusal used to be raised by the parser, `0A000`, before any lookup.
-- @expect-error: the relation is looked up first
-- @expect-message
-- @test
ALTER TABLE nosuch_atie18 RENAME TO other_atie18;
