-- `||` is `text||text`, `text||anynonarray`, `anynonarray||text`, plus the bytea /
-- jsonb / array forms. Two integers match none of them, so Postgres resolves no
-- operator (42883); pglet used to stringify both sides and silently return '12'.
-- @test
-- @expect-error: no integer || integer operator
SELECT 1::int || 2::int;
