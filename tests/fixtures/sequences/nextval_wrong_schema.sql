-- A non-`public` schema qualifier fails loud (42P01) instead of silently dropping
-- the prefix and advancing the bare-named sequence. Matches Postgres, which errors
-- `relation "otherschema.seq_ws" does not exist`.
-- @setup
CREATE SEQUENCE seq_ws;
-- @test
SELECT nextval('otherschema.seq_ws');
-- @expect-error: relation "otherschema.seq_ws" does not exist (42P01)
