-- `date - date → int4` (the day count between two dates, Postgres
-- `pg_operator.dat`). Covers a positive span, the reversed (negative) span, and a
-- same-day zero.
-- @test
SELECT '2026-07-05'::date - '2026-01-01'::date AS span,
       '2020-01-01'::date - '2020-12-31'::date AS neg_span,
       '2024-02-29'::date - '2024-02-29'::date AS same_day;
