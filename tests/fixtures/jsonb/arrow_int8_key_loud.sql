-- Postgres has no `(jsonb, bigint)` `->` signature, only `(jsonb, integer)` and
-- `(jsonb, text)`. pglet accepted an `int8` key as a leniency; it now matches
-- Postgres and fails loud, so a query can't pass here and fail on real Postgres.
-- @test
-- @expect-error: no jsonb -> bigint
SELECT '[1,2,3]'::jsonb -> 1::int8;
