-- `DROP CONSTRAINT` over a missing table is `42P01`. The parser read `CONSTRAINT` as a
-- column name, so the statement was `42601`.
-- @expect-error: the relation is looked up first
-- @expect-message
-- @test
ALTER TABLE nosuch_atie19 DROP CONSTRAINT atie19_pkey;
