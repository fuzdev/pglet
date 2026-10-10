-- nextval on a sequence that does not exist: SQLSTATE 42P01 (undefined_table),
-- Postgres resolving the sequence as a relation.
-- @test
SELECT nextval('seq_does_not_exist');
-- @expect-error: relation does not exist (42P01)
