-- A three-part name whose first part is not the current database is `0A000`, a hard
-- error under `to_regclass` too.
-- @expect-error: cross-database reference
-- @test
SELECT to_regclass('trc_other_db.public.trc');
