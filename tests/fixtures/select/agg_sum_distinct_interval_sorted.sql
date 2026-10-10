-- @test
-- `sum`/`avg(DISTINCT …)` over an `interval` add the distinct values in sort order:
-- `-178000000 years + 100000000 years + 178000000 years` stays in range, where the
-- input order's first partial sum, `178000000 years + 100000000 years`, overflows the
-- month count (`agg_sum_interval_input_order_overflow_loud.sql`). The plain `sum` keeps
-- input order, given here already sorted.
SELECT
  (SELECT sum(DISTINCT x) FROM (VALUES (interval '178000000 years'), (interval '100000000 years'), (interval '-178000000 years')) v(x)) AS sum_d,
  (SELECT avg(DISTINCT x) FROM (VALUES (interval '178000000 years'), (interval '100000000 years'), (interval '-178000000 years')) v(x)) AS avg_d,
  (SELECT sum(x) FROM (VALUES (interval '-178000000 years'), (interval '100000000 years'), (interval '178000000 years')) v(x)) AS sum_sorted_input;
