-- `->` / `->>` exist only over json/jsonb (pglet additionally parses a `text` left
-- operand as JSON). A `text[]` left operand matches no signature, so Postgres is
-- 42883; pglet used to fall through to a silent NULL.
-- @test
-- @expect-error: no text[] -> integer operator
SELECT ARRAY['a','b'] -> 1;
