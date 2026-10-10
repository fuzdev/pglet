-- A cooked default follows the heap, so the table's own 42P07 outranks it. pglet used
-- to answer 42809 here — the `DEFAULT nextval('x')` gate ran inside the column loop,
-- above the table's existence check.
-- @setup
CREATE TABLE dnyet (z INT);
CREATE TABLE dnyet_host (a INT);
-- @test
CREATE TABLE dnyet (b INT DEFAULT nextval('dnyet_host'), c INT);
-- @expect-error: the existing relation outranks the default (42P07)
