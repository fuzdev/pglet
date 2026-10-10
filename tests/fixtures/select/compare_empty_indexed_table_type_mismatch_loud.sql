-- `integer = boolean` has no operator, and Postgres says so at parse time — with no
-- rows to look at. pglet derived the operand types from the runtime `Value`s, so over
-- an **empty** table the gate never fired and the query quietly returned zero rows.
-- With an index on the column that split the two build profiles: the accelerated
-- build lifted the predicate into a probe, whose gate compares *declared* types and
-- did fire, while the stripped build's `Filter(Scan)` saw no rows. The planner now
-- gates on the declared types before index selection, so both profiles agree.
-- @profile-lock
-- @setup
CREATE TABLE t (name TEXT, n INT);
CREATE INDEX t_n ON t (n);
-- @test
-- @expect-error: no integer = boolean operator, even with no rows
SELECT 1 FROM t WHERE n = true;
