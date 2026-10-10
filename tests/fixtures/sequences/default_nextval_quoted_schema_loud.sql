-- The DDL route into the same reader, and the reason a quoted `"PUBLIC"` qualifier was
-- top-severity rather than a merely wrong error: a `DEFAULT nextval('x')` **auto-creates**
-- `x` on pglet, so a qualifier that resolved where Postgres refuses did not just answer
-- differently — the statement *succeeded* and the catalog kept a sequence 18.6 will not
-- create (`42P01` there, at the same statement).
-- @setup
CREATE SEQUENCE s;
-- @test
CREATE TABLE b12 (a INT DEFAULT nextval('"PUBLIC".s2'), v INT);
-- @expect-error: a quoted schema qualifier names no schema (42P01)
