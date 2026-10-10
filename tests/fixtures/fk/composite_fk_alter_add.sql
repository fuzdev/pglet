-- A composite FOREIGN KEY added via ALTER TABLE ... ADD CONSTRAINT builds a real FK
-- (the existing child tuple validates against the parent), then enforces: a later
-- orphan insert whose (a, b) tuple has no parent is rejected with 23503.
-- @profile-lock
-- @setup
CREATE TABLE caa_parent (x INT, y INT, PRIMARY KEY (x, y));
CREATE TABLE caa_child (a INT, b INT);
INSERT INTO caa_parent VALUES (1, 2);
INSERT INTO caa_child VALUES (1, 2);
ALTER TABLE caa_child ADD CONSTRAINT caa_child_ab_fkey FOREIGN KEY (a, b) REFERENCES caa_parent (x, y);

-- @test
-- @expect-error: (5, 6) has no matching parent — rejected by the ALTER-added FK (23503)
INSERT INTO caa_child VALUES (5, 6);
