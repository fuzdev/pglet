-- The plain `sum(interval)` adds in input order, and `178000000 years + 100000000
-- years` overflows the month count before the negative input is reached.
-- @test
-- @expect-error: 22008 interval out of range
SELECT sum(x) FROM (VALUES (interval '178000000 years'), (interval '100000000 years'), (interval '-178000000 years')) v(x);
