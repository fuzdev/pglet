-- `sum` is a fixed catalog accept-list (int2/4/8, float4/8, numeric, interval —
-- plus `money`, which pglet doesn't model). A text operand matches none, so
-- Postgres is 42883; pglet used to skip every non-numeric value and return NULL.
-- @setup
CREATE TABLE sw (name TEXT);
INSERT INTO sw VALUES ('a'), ('b');
-- @test
-- @expect-error: no sum(text)
SELECT SUM(name) FROM sw;
