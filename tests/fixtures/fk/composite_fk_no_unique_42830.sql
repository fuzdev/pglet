-- A composite FOREIGN KEY whose referenced columns are covered by no PK/UNIQUE key
-- on the parent fails loud at CREATE TABLE with 42830 (there is no unique constraint
-- matching given keys for referenced table).
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
-- @setup
CREATE TABLE cnu_parent (x INT, y INT);

-- @test
-- @expect-error: parent has no unique key on (x, y) — 42830
CREATE TABLE cnu_child (a INT, b INT, FOREIGN KEY (a, b) REFERENCES cnu_parent (x, y));
