-- An aggregate in a CHECK constraint is forbidden — Postgres raises 42803
-- (`aggregate functions are not allowed in check constraints`). pglet must match the
-- SQLSTATE (rejected at DDL time), comprehensively matching Postgres's CHECK
-- restrictions.
-- @expect-error: aggregate functions are not allowed in check constraints (42803)
-- @test
CREATE TABLE ag (a INT CHECK (sum(a) > 0));
